//! Handling of parameters and arguments
use crate::error::{self, AddErrors, Errors, Span, Spanned, WithSpan};
use std::collections::HashSet;
use thiserror;

use super::value::{Ident, Value};

#[derive(Debug, PartialEq, Clone)]
pub struct Parameter {
    pub default: Option<Value>,
    pub span: Option<Span>,
}

#[derive(Debug, Default, Clone)]
pub struct Parameters(pub Vec<(Ident, Parameter)>);

pub struct ParametersBuilder<'a, I> {
    interner: &'a mut I,
    parameters: Vec<(Ident, Parameter)>,
}

impl<'a, I> ParametersBuilder<'a, I> {
    pub fn build(self) -> Parameters {
        Parameters(self.parameters)
    }
    pub fn add_static(mut self, name: &str, default: Option<Value>) -> Self
    where
        I: lasso::Interner,
    {
        self.parameters.push((
            Ident(self.interner.get_or_intern(name)),
            Parameter {
                default,
                span: None,
            },
        ));
        self
    }
}

impl Parameters {
    pub fn builder<'a, I>(interner: &'a mut I) -> ParametersBuilder<'a, I> {
        ParametersBuilder {
            interner,
            parameters: Vec::new(),
        }
    }
}

#[derive(Debug, Default)]
pub struct Arguments<V> {
    pub positional: Vec<Spanned<V>>,
    pub key_value: Vec<Spanned<(Ident, V)>>,
}

#[derive(Debug, Clone)]
pub enum Pattern {
    Literal(Value),
    Bind(Ident),
    Wildcard,
}

impl<'a> Pattern {
    pub fn matches(&self, v: &Value) -> bool {
        match self {
            Pattern::Bind(_) => true,
            Pattern::Literal(l) => l == v,
            Pattern::Wildcard => true,
        }
    }
}

pub type PatternArguments<'a> = Arguments<Pattern>;

#[derive(Debug, Default)]
pub struct Patterns(pub Vec<Pattern>);

impl<'a> Patterns {
    pub fn matches(&self, vals: &Vec<Value>) -> bool {
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

impl Parameters {
    /// Bind positional and keyword arguments to parameter slots in declaration
    /// order. Reports arity overflow and unknown/duplicate/repeated keywords.
    /// Unfilled slots are left `None` for the caller to resolve.
    fn bind<V: Clone>(
        &self,
        resolver: &impl lasso::Resolver,
        args: &Arguments<V>,
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
        let mut seen: HashSet<&Ident> = HashSet::new();
        for arg in &args.key_value {
            let (name, value) = &arg.inner;
            let span = arg.span;
            if !seen.insert(name) {
                let _: Option<()> = Err(MatchError::RepeatedKeyword(
                    String::from(resolver.resolve(&name.0)),
                ))
                .with_opt_span(span)
                .add_errors(errors);
            } else if let Some(idx) = self.0.iter().position(|(p, _)| p == name)
            {
                if idx < n {
                    let _: Option<()> = Err(MatchError::DuplicateArgument(
                        String::from(resolver.resolve(&name.0)),
                    ))
                    .with_opt_span(span)
                    .add_errors(errors);
                } else {
                    slots[idx] = Some(value.clone());
                }
            } else {
                let _: Option<()> = Err(MatchError::UnknownKeyword(
                    String::from(resolver.resolve(&name.0)),
                ))
                .with_opt_span(span)
                .add_errors(errors);
            }
        }

        slots
    }

    // Might better be called "resolve" since this is run in the
    // analyser, during evaluation the shape has already been fixed
    pub fn match_arguments<V: Clone + From<Value>>(
        &self,
        resolver: &impl lasso::Resolver,
        args: error::Spanned<&Arguments<V>>,
    ) -> Result<Vec<V>, Errors<MatchError>> {
        let mut errors = Errors::empty();
        let mut slots = self.bind(resolver, args.inner, &mut errors);
        // Required coverage, in declaration order; fill in defaults.
        for (idx, (name, param)) in self.0.iter().enumerate() {
            if slots[idx].is_none() {
                match &param.default {
                    None => {
                        let _: Option<()> = Err(MatchError::MissingRequired(
                            String::from(resolver.resolve(&name.0)),
                        ))
                        .with_opt_span(args.span)
                        .add_errors(&mut errors);
                    }
                    Some(default) => slots[idx] = Some(default.clone().into()),
                }
            }
        }

        errors.ok(slots
            .into_iter()
            .map(|s| s.unwrap_or(Value::default().into()))
            .collect())
    }

    pub fn match_pattern_arguments(
        &self,
        resolver: &impl lasso::Resolver,
        args: &PatternArguments,
    ) -> Result<Patterns, Errors<MatchError>> {
        let mut errors = Errors::empty();
        let slots = self.bind(resolver, args, &mut errors);

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
