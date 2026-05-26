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
    pub span: Span,
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
    fn with_span<S: Into<Span>, Err: From<E>>(
        self,
        span: S,
    ) -> Result<T, Spanned<Err>>;
}

impl<T, E> WithSpan<T, E> for Result<T, E> {
    fn with_span<S: Into<Span>, Err: From<E>>(
        self,
        span: S,
    ) -> Result<T, Spanned<Err>> {
        self.map_err(|error| Spanned {
            span: span.into(),
            inner: error.into(),
        })
    }
}
