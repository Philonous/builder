//! Handling of parameters and arguments
use crate::error::{Spanned, WithSpan};
use std::{
    collections::{HashMap, HashSet},
    ops::Deref,
};
use thiserror;

use super::base::{Ident, Value};

#[derive(Debug, PartialEq, Clone)]
pub struct Parameter<'a> {
    pub default: Option<Value<'a>>,
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
    pub positional: Vec<Value<'a>>,
    pub key_value: Vec<(Ident<'a>, Value<'a>)>,
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
    ) -> Result<HashMap<Ident<'a>, Value<'a>>, Vec<Spanned<MatchError>>> {
        let mut errors = Vec::new();
        let mut out: HashMap<Ident<'a>, Value<'a>> =
            HashMap::with_capacity(self.0.len());

        // 1. Positional: bind what fits; the first `n` params are the bound ones.
        let n = self.0.len().min(args.positional.len());
        for ((name, _), value) in self.0.iter().zip(&args.positional[..n]) {
            out.insert(name.clone(), value.clone());
        }
        if args.positional.len() > self.0.len() {
            errors.push(MatchError::TooManyPositional {
                expected: self.0.len(),
                got: args.positional.len(),
            });
        }

        // 2. Keyword, in source order: repeated / unknown / clashes-with-positional.
        let mut seen: HashSet<&Ident<'a>> = HashSet::new();
        for (name, value) in &args.key_value {
            if !seen.insert(name) {
                // same keyword supplied earlier in the list
                errors.push(MatchError::RepeatedKeyword(name.to_string()));
            } else if !self.0.iter().any(|(p, _)| p == name) {
                errors.push(MatchError::UnknownKeyword(name.to_string()));
            } else if self.0[..n].iter().any(|(p, _)| p == name) {
                // positional + kw
                errors.push(MatchError::DuplicateArgument(name.to_string()));
            } else {
                out.insert(name.clone(), value.clone());
            }
        }

        // 3. Required coverage, in declaration order, fill in defaults
        for (name, param) in &*self.0 {
            if !out.contains_key(name) {
                match &param.default {
                    None => {
                        errors.push(MatchError::MissingRequired(
                            name.to_string(),
                        ));
                    }
                    Some(default) => {
                        out.insert(name.clone(), default.clone());
                    }
                }
            }
        }

        if errors.is_empty() {
            Ok(out)
        } else {
            Err(errors)
        }
    }
}
