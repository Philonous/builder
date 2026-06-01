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

#[derive(Debug, Default)]
pub struct Parameters<'a>(pub CowSlice<'a, (Ident<'a>, Parameter<'a>)>);

#[derive(Debug, Default)]
pub struct ArgumentSchema<'a, V> {
    pub positional: Vec<Spanned<V>>,
    pub key_value: Vec<Spanned<(Ident<'a>, V)>>,
}

pub type Arguments<'a> = ArgumentSchema<'a, Value<'a>>;

#[derive(Debug, Clone)]
pub enum Pattern<'a> {
    Literal(Value<'a>),
    Bind(Ident<'a>),
    Wildcard,
}

impl<'a> Pattern<'a> {
    pub fn matches(&self, v: &Value<'a>) -> bool {
        match self {
            Pattern::Bind(_) => true,
            Pattern::Literal(l) => l == v,
            Pattern::Wildcard => true,
        }
    }
}

pub type PatternArguments<'a> = ArgumentSchema<'a, Pattern<'a>>;

#[derive(Debug, Default)]
pub struct Patterns<'a>(pub Vec<Pattern<'a>>);

impl<'a> Patterns<'a> {
    pub fn matches(&self, vals: &Vec<Value<'a>>) -> bool {
        debug_assert_eq!(self.0.len(), vals.len());
        self.0
            .iter()
            .zip(vals.iter())
            .all(|(pat, val)| pat.matches(val))
    }
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
    /// Bind positional and keyword arguments to parameter slots in declaration
    /// order. Reports arity overflow and unknown/duplicate/repeated keywords.
    /// Unfilled slots are left `None` for the caller to resolve.
    fn bind<V: Clone>(
        &self,
        args: &ArgumentSchema<'a, V>,
        errors: &mut Errors<MatchError>,
    ) -> Vec<Option<V>> {
        let mut slots: Vec<Option<V>> = vec![None; self.0.len()];

        // 1. Positional: bind what fits; the first `n` params are the bound ones.
        let n = self.0.len().min(args.positional.len());
        for (slot, arg) in slots[..n].iter_mut().zip(&args.positional[..n]) {
            *slot = Some(arg.inner.clone());
        }
        if args.positional.len() > self.0.len() {
            let span = args.positional[self.0.len()].span;
            let _: Option<()> = Err(MatchError::TooManyPositional {
                expected: self.0.len(),
                got: args.positional.len(),
            })
            .with_opt_span(span)
            .add_errors(errors);
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
                        .add_errors(errors);
            } else if let Some(idx) = self.0.iter().position(|(p, _)| p == name)
            {
                if idx < n {
                    let _: Option<()> =
                        Err(MatchError::DuplicateArgument(name.to_string()))
                            .with_opt_span(span)
                            .add_errors(errors);
                } else {
                    slots[idx] = Some(value.clone());
                }
            } else {
                let _: Option<()> =
                    Err(MatchError::UnknownKeyword(name.to_string()))
                        .with_opt_span(span)
                        .add_errors(errors);
            }
        }

        slots
    }

    pub fn match_arguments(
        &self,
        args: &Arguments<'a>,
    ) -> Result<Vec<Value<'a>>, Errors<MatchError>> {
        let mut errors = Errors::empty();
        let mut slots = self.bind(args, &mut errors);

        // Required coverage, in declaration order; fill in defaults.
        for (idx, (name, param)) in self.0.iter().enumerate() {
            if slots[idx].is_none() {
                match &param.default {
                    None => {
                        let _: Option<()> =
                            Err(MatchError::MissingRequired(name.to_string()))
                                .without_span()
                                .add_errors(&mut errors);
                    }
                    Some(default) => slots[idx] = Some(default.clone()),
                }
            }
        }

        if errors.0.is_empty() {
            Ok(slots.into_iter().map(Option::unwrap).collect())
        } else {
            Err(errors)
        }
    }

    pub fn match_pattern_arguments(
        &self,
        args: &PatternArguments<'a>,
    ) -> Result<Patterns<'a>, Errors<MatchError>> {
        let mut errors = Errors::empty();
        let slots = self.bind(args, &mut errors);

        if !errors.0.is_empty() {
            return Err(errors);
        }

        // An unmentioned parameter is unconstrained, hence Wildcard. Defaults
        // and required-ness are call-site notions, irrelevant to a match clause.
        Ok(Patterns(
            slots
                .into_iter()
                .map(|slot| slot.unwrap_or(Pattern::Wildcard))
                .collect(),
        ))
    }
}
