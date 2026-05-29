use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl From<chumsky::span::SimpleSpan> for Span {
    fn from(value: chumsky::span::SimpleSpan) -> Self {
        Span {
            start: value.start,
            end: value.end,
        }
    }
}

#[derive(Debug, Error, Clone)]
pub struct Spanned<I> {
    pub span: Option<Span>,
    pub inner: I,
}

impl<I: PartialEq> PartialEq for Spanned<I> {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl<I> Spanned<I> {
    fn map<O, F>(self, f: F) -> Spanned<O>
    where
        F: FnOnce(I) -> O,
    {
        Spanned {
            span: self.span,
            inner: f(self.inner),
        }
    }

    pub fn convert<T>(self) -> Spanned<T>
    where
        T: From<I>,
    {
        self.map(Into::into)
    }
}

impl<E> std::fmt::Display for Spanned<E>
where
    E: std::fmt::Display,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.inner)
    }
}

pub trait WithSpan<T, E> {
    fn with_span_from<S: Into<Span>, Err: From<E>>(
        self,
        span: S,
    ) -> Result<T, Spanned<Err>>;

    fn with_span<S: Into<Span>>(self, span: S) -> Result<T, Spanned<E>>;

    fn without_span(self) -> Result<T, Spanned<E>>;
}

impl<T, E> WithSpan<T, E> for Result<T, E> {
    fn with_span_from<S: Into<Span>, Err: From<E>>(
        self,
        span: S,
    ) -> Result<T, Spanned<Err>> {
        self.map_err(|error| Spanned {
            span: Some(span.into()),
            inner: error.into(),
        })
    }
    fn with_span<S: Into<Span>>(self, span: S) -> Result<T, Spanned<E>> {
        self.map_err(|error| Spanned {
            span: Some(span.into()),
            inner: error,
        })
    }
    fn without_span(self) -> Result<T, Spanned<E>> {
        self.map_err(|error| Spanned {
            span: None,
            inner: error.into(),
        })
    }
}

#[derive(Debug)]
pub struct Errors<E>(pub Vec<Spanned<E>>);

impl<E> Errors<E> {
    pub fn empty() -> Self {
        Errors(Vec::new())
    }
}

pub trait IntoErrors<E> {
    fn drain_into(self, errors: &mut Vec<Spanned<E>>);
}

impl<E, F: Into<E>> IntoErrors<E> for Spanned<F> {
    fn drain_into(self, errors: &mut Vec<Spanned<E>>) {
        errors.push(self.map(|f| f.into()));
    }
}

impl<E, F: Into<E>> IntoErrors<E> for Vec<Spanned<F>> {
    fn drain_into(self, errors: &mut Vec<Spanned<E>>) {
        errors.extend(self.into_iter().map(|f| f.map(Into::into)));
    }
}

impl<E, F: Into<E>> IntoErrors<E> for Errors<F> {
    fn drain_into(self, errors: &mut Vec<Spanned<E>>) {
        self.0.drain_into(errors);
    }
}

impl<E> Errors<E> {
    pub fn append<F: IntoErrors<E>>(&mut self, f: F) {
        f.drain_into(&mut self.0);
    }
}

pub trait AddErrors<ES> {
    type Res;
    fn add_errors(self, errors: &mut Errors<ES>) -> Option<Self::Res>;
}

impl<T, ES, E: IntoErrors<ES>> AddErrors<ES> for Result<T, E> {
    type Res = T;
    fn add_errors(self, errors: &mut Errors<ES>) -> Option<T> {
        match self {
            Ok(ok) => Some(ok),
            Err(e) => {
                e.drain_into(&mut errors.0);
                None
            }
        }
    }
}

// Try the expression, on error append errors, then return the errors
macro_rules! try_errors {
    ($errors:ident, $v:expr) => {
        match $v {
            Ok(ok) => (),
            Err(e) => {
                $errors.append(e)
                return Err($errors)
            }
        }
    };
}

pub(crate) use try_errors;
