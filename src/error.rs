use std::ops::Range;

use ariadne::{Color, Config, IndexType, Label, Report, ReportKind};
use chumsky::error::Rich;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn to_range(&self) -> Range<usize> {
        self.start..self.end
    }
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

    pub fn spanned<T: Into<Span>>(inner: I, span: T) -> Self {
        Self {
            inner,
            span: Some(span.into()),
        }
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
        span: Option<S>,
    ) -> Result<T, Spanned<Err>>;

    fn with_span<S: Into<Span>>(self, span: S) -> Result<T, Spanned<E>>;

    fn with_opt_span(self, span: Option<Span>) -> Result<T, Spanned<E>>;

    fn without_span(self) -> Result<T, Spanned<E>>;
}

impl<T, E> WithSpan<T, E> for Result<T, E> {
    fn with_span_from<S: Into<Span>, Err: From<E>>(
        self,
        span: Option<S>,
    ) -> Result<T, Spanned<Err>> {
        self.map_err(|error| Spanned {
            span: span.map(|s| s.into()),
            inner: error.into(),
        })
    }
    fn with_span<S: Into<Span>>(self, span: S) -> Result<T, Spanned<E>> {
        self.map_err(|error| Spanned {
            span: Some(span.into()),
            inner: error,
        })
    }
    fn with_opt_span(self, span: Option<Span>) -> Result<T, Spanned<E>> {
        self.map_err(|error| Spanned { span, inner: error })
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
    /// New empty error list
    pub fn empty() -> Self {
        Errors(Vec::new())
    }

    /// Return Ok(val) if the list of errors is empty, otherwise Err(errors)
    pub fn ok<V>(self, v: V) -> Result<V, Self> {
        if self.0.len() > 0 {
            Err(self)
        } else {
            Ok(v)
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Spanned<E>> {
        self.0.iter()
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

pub trait ToRange {
    fn to_range(&self) -> Option<Range<usize>>;
}

impl<'a, T> ToRange for Rich<'a, T> {
    fn to_range(&self) -> Option<Range<usize>> {
        Some(self.span().into_range())
    }
}

impl<T> ToRange for Spanned<T> {
    fn to_range(&self) -> Option<Range<usize>> {
        self.span.map(|s| s.to_range())
    }
}

pub trait ToMessage {
    fn to_message(&self) -> String;
}

impl<'a, T: std::fmt::Display> ToMessage for Rich<'a, T> {
    fn to_message(&self) -> String {
        self.reason().to_string()
    }
}

impl<'a, T: std::fmt::Display> ToMessage for Spanned<T> {
    fn to_message(&self) -> String {
        self.inner.to_string()
    }
}

pub fn report<'a, E: ToMessage + ToRange>(
    err: &E,
) -> Report<'a, ((), Range<usize>)> {
    let range = err.to_range();
    // Report::build needs *a* location even with no label; the file start is the
    // least-misleading default.
    let location = ((), range.clone().unwrap_or(0..0));

    let mut builder = Report::build(ReportKind::Error, location)
        .with_config(Config::new().with_index_type(IndexType::Byte))
        .with_code(3)
        .with_message(err.to_message());

    if let Some(range) = range {
        builder = builder.with_label(
            Label::new(((), range))
                .with_message(err.to_message())
                .with_color(Color::Red),
        );
    }

    builder.finish()
}
