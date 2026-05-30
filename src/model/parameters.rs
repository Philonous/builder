//! Handling of parameters and arguments
use crate::error::{AddErrors, Errors, Span, Spanned, WithSpan};
use std::{
    collections::{HashMap, HashSet},
    ops::Deref,
};
use thiserror;

use super::base::{Ident, Value};

#[derive(Debug, PartialEq, Clone)]
pub struct Parameter<'a> {
    pub default: Option<Value<'a>>,
    pub span: Option<Span>,
}

// Custom enum to deal with lifetime variance
// Default cow is invariant in lifetimes that appear in the
// wrapped type, because it needs to hold an associated type parameter
#[derive(Debug)]
pub enum CowSlice<'a, T> {
    Borrowed(&'a [T]),
    Owned(Vec<T>),
}

impl<'a, T> Default for CowSlice<'a, T> {
    fn default() -> Self {
        Self::Borrowed(&[])
    }
}

impl<'a, T> Deref for CowSlice<'a, T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Borrowed(s) => *s,
            Self::Owned(v) => &**v,
        }
    }
}

// Expected parameters (e.g. of a function), in order
#[derive(Debug, Default)]
pub struct Parameters<'a>(pub CowSlice<'a, (Ident<'a>, Parameter<'a>)>);

#[derive(Debug, Default)]
pub struct Arguments<'a> {
    pub positional: Vec<Spanned<Value<'a>>>,
    pub key_value: Vec<Spanned<(Ident<'a>, Value<'a>)>>,
}

#[derive(Debug, thiserror::Error)]
pub enum MatchError {
    #[error(
        "too many positional arguments: expected at most {expected}, got {got}"
    )]
    TooManyPositional { expected: usize, got: usize },
    #[error("unknown keyword argument `{0}`")]
    UnknownKeyword(String),
    #[error("argument `{0}` supplied both positionally and by keyword")]
    DuplicateArgument(String),
    #[error("keyword argument `{0}` supplied more than once")]
    RepeatedKeyword(String),
    #[error("missing required argument `{0}`")]
    MissingRequired(String),
}

impl<'a> Parameters<'a> {
    pub fn match_arguments(
        &self,
        args: &Arguments<'a>,
    ) -> Result<HashMap<Ident<'a>, Value<'a>>, Errors<MatchError>> {
        let mut errors = Errors::empty();
        let mut out: HashMap<Ident<'a>, Value<'a>> =
            HashMap::with_capacity(self.0.len());

        // 1. Positional: bind what fits; the first `n` params are the bound ones.
        let n = self.0.len().min(args.positional.len());
        for ((name, _), arg) in self.0.iter().zip(&args.positional[..n]) {
            out.insert(name.clone(), arg.inner.clone());
        }
        if args.positional.len() > self.0.len() {
            let span = args.positional[self.0.len()].span;
            let _: Option<()> = Err(MatchError::TooManyPositional {
                expected: self.0.len(),
                got: args.positional.len(),
            })
            .with_opt_span(span)
            .add_errors(&mut errors);
        }

        // 2. Keyword, in source order: repeated / unknown / clashes-with-positional.
        let mut seen: HashSet<&Ident<'a>> = HashSet::new();
        for arg in &args.key_value {
            let (name, value) = &arg.inner;
            let span = arg.span;
            if !seen.insert(name) {
                let _: Option<()> =
                    Err(MatchError::RepeatedKeyword(name.to_string()))
                        .with_opt_span(span)
                        .add_errors(&mut errors);
            } else if !self.0.iter().any(|(p, _)| p == name) {
                let _: Option<()> =
                    Err(MatchError::UnknownKeyword(name.to_string()))
                        .with_opt_span(span)
                        .add_errors(&mut errors);
            } else if self.0[..n].iter().any(|(p, _)| p == name) {
                let _: Option<()> =
                    Err(MatchError::DuplicateArgument(name.to_string()))
                        .with_opt_span(span)
                        .add_errors(&mut errors);
            } else {
                out.insert(name.clone(), value.clone());
            }
        }

        // 3. Required coverage, in declaration order, fill in defaults
        for (name, param) in &*self.0 {
            if !out.contains_key(name) {
                match &param.default {
                    None => {
                        let _: Option<()> =
                            Err(MatchError::MissingRequired(name.to_string()))
                                .without_span()
                                .add_errors(&mut errors);
                    }
                    Some(default) => {
                        out.insert(name.clone(), default.clone());
                    }
                }
            }
        }

        if errors.0.is_empty() {
            Ok(out)
        } else {
            Err(errors)
        }
    }
}
