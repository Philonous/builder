//! Handling of parameters and arguments

use std::collections::HashMap;
use thiserror;

use super::base::{Ident, Value};

struct Parameter<'a> {
    default: Option<Value<'a>>,
}

// Expected parameters (e.g. of a function), in order
pub struct Parameters<'a>(Vec<(Ident<'a>, Parameter<'a>)>);

pub struct Arguments<'a> {
    positional: Vec<Value<'a>>,
    key_value: HashMap<Ident<'a>, Value<'a>>,
}

#[derive(Debug, thiserror::Error)]
pub enum MatchError<'a> {
    #[error(
        "too many positional arguments: expected at most {expected}, got {got}"
    )]
    TooManyPositional { expected: usize, got: usize },
    #[error("unknown keyword argument `{0}`")]
    UnknownKeyword(Ident<'a>),
    #[error("argument `{0}` supplied both positionally and by keyword")]
    DuplicateArgument(Ident<'a>),
    #[error("keyword argument `{0}` supplied more than once")]
    RepeatedKeyword(Ident<'a>),
    #[error("missing required argument `{0}`")]
    MissingRequired(Ident<'a>),
}

/// One or more mismatches. Always non-empty.
#[derive(Debug)]
pub struct MatchErrors<'a>(pub Vec<MatchError<'a>>);

impl std::fmt::Display for MatchErrors<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, e) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{e}")?;
        }
        Ok(())
    }
}

impl std::error::Error for MatchErrors<'_> {}

impl<'a> Parameters<'a> {
    pub fn match_arguments(
        &self,
        args: &Arguments<'a>,
    ) -> Result<HashMap<Ident<'a>, Value<'a>>, MatchErrors<'a>> {
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

        // 2. Keyword, in source order: unknown / clashes-with-positional / repeated.
        for (name, value) in &args.key_value {
            if !self.0.iter().any(|(p, _)| p == name) {
                errors.push(MatchError::UnknownKeyword(name.clone()));
            } else if self.0[..n].iter().any(|(p, _)| p == name) {
                // positional + kw
                errors.push(MatchError::DuplicateArgument(name.clone()));
            } else if out.insert(name.clone(), value.clone()).is_some() {
                // kw twice
                errors.push(MatchError::RepeatedKeyword(name.clone()));
            }
        }

        // 3. Required coverage, in declaration order, fill in defaults
        for (name, param) in &self.0 {
            if !out.contains_key(name) {
                match &param.default {
                    None => {
                        errors.push(MatchError::MissingRequired(name.clone()));
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
            Err(MatchErrors(errors))
        }
    }
}
