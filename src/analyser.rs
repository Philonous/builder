use super::model::eval;
use super::model::parameters;
use super::model::target;
use super::parser::parser;
use crate::error::AddErrors;
use crate::error::Errors;
use crate::error::WithSpan;
use crate::{
    common::Ident,
    error::{self, Spanned},
    model::eval::Slots,
    parser::string::QuoteError,
};

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("variable {0} was already declared")]
    DuplicateVariable(String),
    #[error("variable {0} used before being declared")]
    UndefinedVariable(String),
    #[error("{0}")]
    QuoteError(#[from] QuoteError),
    #[error("unknown declaration {0}")]
    UnknownDecl(String),
    #[error("unknown target type {0}")]
    UnknownTargetType(String),
    #[error("{0}")]
    ParamMatchError(#[from] parameters::MatchError),
    #[error("positional parameter {0} after key-value")]
    ParamBareAfterKv(String),
}

impl<'a> Slots<'a> {
    // Create a new variable in the current scope
    fn new_variable(&mut self, name: &Ident<'a>) -> Result<eval::Var, Error> {
        // Check if the variable is already declared
        match self.find_variable(name) {
            Some(_) => {
                return Err(Error::DuplicateVariable(name.to_string()));
            }
            None => (),
        }
        // Create a new slot for the variable
        self.0.push(eval::Slot {
            name: name.clone(),
            value: None,
        });

        // Create variable referencing slot
        Ok(eval::Var {
            // New variables are always created in the *current*
            // scope, so nesting is 0
            nesting: 0,
            slot: self.0.len() - 1,
        })
    }

    fn find_variable(&self, name: &Ident<'a>) -> Option<usize> {
        self.0.iter().position(|slot| &slot.name == name)
    }
}

struct Scopes<'a, 'b>(&'b mut Vec<Slots<'a>>);

impl<'a, 'b> Scopes<'a, 'b> {
    /// Push a fresh scope; the returned guard pops it when it leaves.
    fn enter(&mut self) -> ScopeGuard<'a, '_> {
        self.0.push(Slots(Vec::new()));
        ScopeGuard(Scopes(&mut *self.0)) // reborrow, so the guard can't outlive us
    }
}

struct ScopeGuard<'a, 'b>(Scopes<'a, 'b>);

impl<'a, 'b> ScopeGuard<'a, 'b> {
    fn new_variable(&mut self, name: &Ident<'a>) -> Result<eval::Var, Error> {
        // Last element is the current scope where we want to create
        // the new variable

        self.0
             .0
            .last_mut()
            .expect("ScopeGuard: empty")
            .new_variable(name)
    }
    fn find_variable(&self, name: &Ident<'a>) -> Result<eval::Var, Error> {
        // Iterate over all scopes (inner to outer). For each scope,
        // look for the variable. Return the innermost matching scope
        // + the slot
        self.0
             .0
            .iter()
            .rev()
            .enumerate()
            .find_map(|(nesting, scope)| {
                scope
                    .find_variable(name)
                    .map(|slot| eval::Var { slot, nesting })
            })
            .ok_or_else(|| Error::UndefinedVariable(name.to_string()))
    }

    /// Success path: pop and take ownership of the scope (e.g. to attach to a block).
    fn detach(mut self) -> Slots<'a> {
        let Scopes(vec) = &mut self.0;
        let slots = vec.pop().expect("guard guarantees a live scope");
        std::mem::forget(self); // skip Drop so we don't pop twice
        slots
    }

    fn nested_scope(&mut self) -> &mut Scopes<'a, 'b> {
        &mut self.0
    }
}

impl<'a, 'b> Drop for ScopeGuard<'a, 'b> {
    fn drop(&mut self) {
        let Scopes(vec) = &mut self.0;
        vec.pop(); // error/early-return path
    }
}

impl<'a> parser::Lit<'a> {
    pub fn resolve<'b>(self) -> Result<eval::Value<'a>, Error> {
        Ok(match self {
            parser::Lit::String(s) => eval::Value::String(s.unquote()?),
        })
    }
}

impl<'a> parser::Expr<'a> {
    pub fn resolve<'b>(
        self,
        scopes: &mut ScopeGuard<'a, 'b>,
    ) -> Result<eval::Expr<'a>, Spanned<Error>> {
        let span = self.0.span.into();
        let kind = match self.0.inner {
            parser::ExprKind::Lit(lit) => {
                eval::ExprKind::Lit(lit.resolve().with_span_from(span)?)
            }

            parser::ExprKind::Ident(name) => scopes
                .find_variable(&name)
                .map(eval::ExprKind::Var)
                .with_span(span)?,
            parser::ExprKind::Command(cmd) => cmd
                .resolve_strings()
                .map(|parser::Command { executable, args }| {
                    eval::ExprKind::Command { executable, args }
                })
                .with_span_from(span)?,
            parser::ExprKind::FunApp(ident, exprs) => unimplemented!(),
        };
        Ok(eval::Expr { kind, span })
    }
}

impl<'a> parser::LitArguments<'a> {
    pub fn resolve<'b>(
        self,
    ) -> Result<parameters::Arguments<'a>, Spanned<Error>> {
        let mut positional = Vec::new();
        let mut key_value = Vec::new();
        // Resolve literals in argument position and check that
        // positional arguments don't occur after key-value arguments
        for arg in self.0 {
            let span = arg.span;
            match arg.inner {
                parser::LitArgument::Bare(lit) => {
                    let val = lit.resolve().with_span(span)?;
                    if key_value.len() > 0 {
                        return Err(Error::ParamBareAfterKv(val.to_string()))
                            .with_span(arg.span);
                    }
                    positional.push(Spanned::spanned(val, arg.span));
                }
                parser::LitArgument::Named { name, value } => {
                    let val = value.resolve().with_span(span)?;
                    key_value.push(Spanned::spanned((name, val), arg.span));
                }
            }
        }
        Ok(parameters::Arguments {
            positional,
            key_value,
        })
    }
}

impl<'a> parser::Patterns<'a> {
    pub fn resolve<'b>(
        self,
    ) -> Result<parameters::PatternArguments<'a>, Errors<Error>> {
        let mut errors = error::Errors::empty();
        let mut positional = Vec::new();
        let mut key_value = Vec::new();

        for arg in self.0 {
            let span = arg.span;
            match arg.inner {
                // Special case:
                // Treat a bare variable as a pun - foo => foo = foo
                parser::Pattern::Bare(parser::PatternKind::Bind(var)) => {
                    key_value.push(Spanned::spanned(
                        (var, parameters::Pattern::Bind(var)),
                        span,
                    ));
                }
                // Bare patterns
                parser::Pattern::Bare(pattern) => {
                    // Check that bare patterns don't occur after named
                    if key_value.len() > 0 {
                        let _: Option<()> =
                            Err(Error::ParamBareAfterKv(pattern.to_string()))
                                .with_span(arg.span)
                                .add_errors(&mut errors);
                    }

                    let res = match pattern {
                        parser::PatternKind::Lit(lit) => {
                            let val = lit
                                .resolve()
                                .with_span(span)
                                .add_errors(&mut errors)
                                .unwrap_or(eval::Value::Unit);
                            parameters::Pattern::Literal(val)
                        }

                        // Bare variable is already handled as punning
                        // case, not here, so we can do the no-bare
                        // check once before all patterns
                        parser::PatternKind::Bind(ident) => unreachable!(
                            "bare variable pattern already handled"
                        ),
                    };

                    positional.push(Spanned::spanned(res, arg.span));
                }

                // Named patterns (path = "backend/app")
                parser::Pattern::Named {
                    name: name,
                    pattern,
                } => {
                    let res = match pattern {
                        parser::PatternKind::Lit(lit) => {
                            let val = lit
                                .resolve()
                                .with_span(span)
                                .add_errors(&mut errors)
                                .unwrap_or(eval::Value::Unit);
                            parameters::Pattern::Literal(val)
                        }
                        parser::PatternKind::Bind(var) => {
                            parameters::Pattern::Bind(var)
                        }
                    };
                    key_value.push(Spanned::spanned((name, res), span));
                }
            }
        }

        let patterns = parameters::ArgumentSchema {
            positional,
            key_value,
        };
        if errors.0.len() > 0 {
            Err(errors)
        } else {
            Ok(patterns)
        }
    }
}

impl<'a> parser::Block<'a> {
    pub fn resolve<'scopes>(
        self,
        outer: &mut Scopes<'a, 'scopes>,
    ) -> Result<eval::Block<'a>, Spanned<Error>> {
        let mut this = outer.enter();
        let mut resolved: Vec<eval::Statement<'a>> =
            Vec::with_capacity(self.0.len());
        for statement in self.0 {
            let stmt = match statement {
                parser::Statement::Let(ident, expr) => {
                    // Resolve expression first so it sees outer scope
                    let expr = expr.resolve(&mut this)?;
                    // Now create variable in inner scope
                    let var = this
                        .new_variable(&ident.inner)
                        .with_span(ident.span)?;

                    eval::Statement::Let {
                        var,
                        span: ident.span.into(),
                        expr,
                    }
                }
                parser::Statement::Expr(expr) => {
                    eval::Statement::Expr(expr.resolve(&mut this)?)
                }
            };
            resolved.push(stmt);
        }

        Ok(eval::Block {
            statements: resolved,
            empty_slots: this.detach(),
        })
    }
}

// All decls are targets for now
impl<'a> parser::Decl<'a> {
    pub fn resolve_target<'scopes>(
        kind: &'a target::TargetType<'a>,
        outer: &mut Scopes<'a, 'scopes>,
        parameters: parameters::Patterns<'a>,
        block: parser::Block<'a>,
    ) -> Result<target::Target<'a>, Errors<Error>> {
        let mut this = outer.enter();
        let mut errors: Errors<Error> = Errors::empty();

        // Add Arguments to the current scope
        // Maybe we should add a namespace prefix, like "target:"
        for (name, _value) in kind.parameters.0.deref().iter() {
            this.new_variable(&name);
        }

        // Resolve the command block
        let block = block
            .resolve(this.nested_scope())
            .add_errors(&mut errors)
            .unwrap_or_default();

        let slots = this.detach();

        let name = match &parameters.0[kind.name_is] {
            parameters::Pattern::Literal(value) => Some(value.to_string()),
            _ => None,
        };

        let target = target::Target {
            kind, //
            parameters,
            name, // TODO
            commands: block,
            slots,
        };

        if errors.0.len() > 0 {
            Err(errors)
        } else {
            Ok(target)
        }
    }

    pub fn resolve<'scopes>(
        self,
        outer: &mut Scopes<'a, 'scopes>,
        target_types: &'a [target::TargetType<'a>],
    ) -> Result<target::Target<'a>, Errors<Error>> {
        let mut errors: Errors<Error> = Errors::empty();

        let target: target::Target = match self.header {
            // @file style target declaration
            parser::DeclHeader::AtDecl(s) => {
                let kind = &target::TARGET_TYPE_FILE;
                let patterns = vec![
                    (parameters::Pattern::Literal(eval::Value::String(
                        Cow::Borrowed(s),
                    ))),
                ];

                parser::Decl::resolve_target(
                    kind,
                    outer,
                    parameters::Patterns(patterns),
                    self.definition,
                )
                .add_errors(&mut errors)
                .unwrap_or_default()
            }
            // Explicit declaration
            parser::DeclHeader::Explicit {
                decl,
                // TODO: handle decl args
                decl_args,
                name,
                pattern,
            } => match decl.inner {
                Ident("target") => {
                    // Look up target type from list
                    let target_type = target_types
                        .iter()
                        .find(|tt| tt.name == name.inner)
                        .ok_or(Error::UnknownTargetType(name.inner.to_string()))
                        .with_span(name.span)
                        .add_errors(&mut errors);

                    // Resolve pattern
                    let pattern = pattern.resolve().add_errors(&mut errors);

                    match target_type.zip(pattern) {
                        // Previous error, skip checking pattern
                        None => {
                            // Errors must already have been set, so
                            // we simply return a dummy variable
                            Default::default()
                        }

                        // Match patterns to parameters
                        Some((target_type, pattern)) => {
                            let pattern = target_type
                                .parameters
                                .match_pattern_arguments(&pattern)
                                .add_errors(&mut errors);
                            match pattern {
                                None => Default::default(),
                                // We have collected all the data, now
                                // we can resolve the targets block
                                Some(args) => parser::Decl::resolve_target(
                                    target_type,
                                    outer,
                                    args,
                                    self.definition,
                                )
                                .add_errors(&mut errors)
                                .unwrap_or_default(),
                            }
                        }
                    }
                }
                // Unknown decl types
                Ident(name) => {
                    let _: Option<()> =
                        Err(Error::UnknownDecl(name.to_string()))
                            .with_span(decl.span)
                            .add_errors(&mut errors);
                    Default::default()
                }
            },
        };
        if errors.0.len() > 0 {
            Err(errors)
        } else {
            Ok(target)
        }
        // Target may want to set special parameters, e.g. resolved path

        // Ok(target::Target { kind: self })
    }
}

static TARGET_TYPES: &[target::TargetType<'static>] =
    &[target::TARGET_TYPE_FILE, target::TARGET_TYPE_TASK];

impl<'a> parser::Ast<'a> {
    pub fn resolve(self) -> Result<Vec<target::Target<'a>>, Errors<Error>> {
        let mut errors = Errors::empty();
        let mut decls = Vec::with_capacity(self.decls.len());
        let mut scopes = Vec::new();

        for decl in self.decls {
            decls.push(
                decl.resolve(&mut Scopes(&mut scopes), TARGET_TYPES)
                    .add_errors(&mut errors)
                    .unwrap_or_default(),
            );
        }

        if errors.0.len() > 0 {
            Err(errors)
        } else {
            Ok(decls)
        }
    }
}
