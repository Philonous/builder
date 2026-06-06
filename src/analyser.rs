use super::model::decl;
use super::model::decl::Decls;
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
use std::ops::Deref;

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
    #[error("positional parameter after key-value")]
    ParamBareAfterKv,
    #[error("function {0} is not known")]
    FunNotFound(String),
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
            .rev() // Last element is innermost scope
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

    /// Add a new nested scope
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

pub struct Ctx<'a> {
    pub functions: &'a [eval::Fun<'a>], // constants
}

impl<'a> parser::Expr<'a> {
    fn resolve<'b>(
        self,
        ctx: &Ctx<'a>,
        scopes: &mut ScopeGuard<'a, 'b>,
    ) -> Result<eval::Expr<'a>, Errors<Error>> {
        let mut errors = Errors::empty();
        let span = self.0.span.into();
        let kind = match self.0.inner {
            parser::ExprKind::Lit(lit) => eval::ExprKind::Lit(
                lit.resolve()
                    .with_span(span)
                    .add_errors(&mut errors)
                    .unwrap_or_default(),
            ),

            parser::ExprKind::Ident(name) => scopes
                .find_variable(&name)
                .map(eval::ExprKind::Var)
                .with_span(span)
                .add_errors(&mut errors)
                .unwrap_or_default(),
            parser::ExprKind::Command(cmd) => cmd
                .resolve_strings()
                .map(|parser::Command { executable, args }| {
                    eval::ExprKind::Command { executable, args }
                })
                .with_span(span)
                .add_errors(&mut errors)
                .unwrap_or_default(),
            parser::ExprKind::FunApp(ident, args) => {
                (|| {
                    let fun =
                        match ctx.functions.iter().find(|f| f.name == ident) {
                            None => Err(Spanned::spanned(
                                Error::FunNotFound(ident.to_string()),
                                span,
                            ))
                            .add_errors(&mut errors),
                            Some(f) => Some(f),
                        }?;
                    let args_separate =
                        args.resolve(ctx, scopes).add_errors(&mut errors)?;

                    let args = fun
                        .parameters
                        .match_arguments(&args_separate)
                        .add_errors(&mut errors)?;
                    Some(eval::ExprKind::FunApp { fun, args })

                    // args.
                })()
                .unwrap_or_default()
            }
        };
        errors.ok(eval::Expr {
            kind,
            span: Some(span),
        })
    }
}

impl<'a> parser::Arguments<'a> {
    fn resolve<'b>(
        self,
        ctx: &Ctx<'a>,
        scopes: &mut ScopeGuard<'a, 'b>,
    ) -> Result<parameters::Arguments<'a, eval::Expr<'a>>, Errors<Error>> {
        let mut errors = error::Errors::empty();
        let mut positional = Vec::new();
        let mut key_value = Vec::new();
        // Resolve literals in argument position and check that
        // positional arguments don't occur after key-value arguments
        for arg in self.0 {
            match arg.inner {
                parser::Argument::Bare(expr) => {
                    let expr =
                        expr.resolve(ctx, scopes).add_errors(&mut errors);
                    if key_value.len() > 0 {
                        let _: Option<()> = Err(Spanned::spanned(
                            Error::ParamBareAfterKv,
                            arg.span,
                        ))
                        .add_errors(&mut errors);
                    } else {
                        expr.map(|expr| {
                            positional.push(Spanned::spanned(expr, arg.span))
                        });
                    }
                }
                parser::Argument::Named { name, expr } => {
                    let expr =
                        expr.resolve(ctx, scopes).add_errors(&mut errors);
                    expr.map(|expr| {
                        key_value.push(Spanned::spanned((name, expr), arg.span))
                    });
                }
            }
        }

        errors.ok(parameters::Arguments {
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
                // Treat a bare variable pattern as a pun,
                // e.g. `foo` => `foo = foo`
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
                        let _: Option<()> = Err(Error::ParamBareAfterKv)
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

                        // Bare variable pattern is already handled as
                        // special case
                        parser::PatternKind::Bind(_) => unreachable!(
                            "bare variable pattern already handled"
                        ),
                    };

                    positional.push(Spanned::spanned(res, arg.span));
                }

                // Named patterns (e.g. path = "backend/app")
                parser::Pattern::Named { name, pattern } => {
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

        let patterns = parameters::Arguments {
            positional,
            key_value,
        };

        errors.ok(patterns)
    }
}

impl<'a> parser::Block<'a> {
    fn resolve<'scopes>(
        self,
        ctx: &Ctx<'a>,
        outer: &mut Scopes<'a, 'scopes>,
    ) -> Result<eval::Block<'a>, Errors<Error>> {
        let mut errors = Errors::empty();
        let mut this = outer.enter();
        let mut resolved: Vec<eval::Statement<'a>> =
            Vec::with_capacity(self.0.len());
        for statement in self.0 {
            let stmt = match statement {
                parser::Statement::Let(ident, expr) => {
                    // let var = expr
                    // Resolve expression first so it sees outer scope
                    // Ordering matters here, we don't want the
                    // expression to see the variable it initialises
                    let expr =
                        expr.resolve(ctx, &mut this).add_errors(&mut errors);
                    // Now create variable in inner scope
                    let var = this
                        .new_variable(&ident.inner)
                        .with_span(ident.span)
                        .add_errors(&mut errors);
                    // Create resolved statement if both var and expr are `Some`
                    var.zip(expr).map(|(var, expr)| eval::Statement::Let {
                        var,
                        span: ident.span.into(),
                        expr,
                    })
                }
                parser::Statement::Expr(expr) => expr
                    .resolve(ctx, &mut this)
                    .add_errors(&mut errors)
                    .map(eval::Statement::Expr),
            };
            // Add statement if it resolved, if any return an error,
            // we don't return the list at all, so we never return a
            // partia list
            stmt.map(|stmt| resolved.push(stmt));
        }

        errors.ok(eval::Block {
            statements: resolved,
            empty_slots: this.detach(),
        })
    }
}

impl<'a> parser::Decl<'a> {
    fn resolve_target<'scopes>(
        ctx: &Ctx<'a>,
        kind: &'a target::TargetType<'a>,
        outer: &mut Scopes<'a, 'scopes>,
        parameters: parameters::Patterns<'a>,
        block: parser::Block<'a>,
        // We only have targets as decls for now
    ) -> Result<target::Target<'a>, Errors<Error>> {
        // target Constr(patter1, pattern2, ...)

        let mut this = outer.enter();
        let mut errors: Errors<Error> = Errors::empty();

        // Resolve target patterns
        let name = match &parameters.0[kind.name_is] {
            parameters::Pattern::Literal(value) => Some(value.to_string()),
            _ => None,
        };

        // Add Arguments to the current scope
        // (As defined by the target type)
        // Maybe we should add a namespace prefix, like "target:"
        for (name, param) in kind.parameters.0.deref().iter() {
            this.new_variable(&name)
                .with_opt_span(param.span)
                .add_errors(&mut errors);
        }

        // Resolve the command block
        let block = block
            .resolve(ctx, this.nested_scope())
            .add_errors(&mut errors)
            .unwrap_or_default();

        let slots = this.detach();

        let target = target::Target {
            kind, //
            parameters,
            name, // TODO
            commands: block,
            slots,
        };

        errors.ok(target)
    }

    fn resolve<'scopes>(
        self,
        ctx: &Ctx<'a>,
        outer: &mut Scopes<'a, 'scopes>,
        target_types: &'a [target::TargetType<'a>],
    ) -> Result<target::Target<'a>, Errors<Error>> {
        let mut errors: Errors<Error> = Errors::empty();

        let target = match self {
            parser::Decl::Target { args, ident, block } => match ident {
                parser::TargetIdent::AtTarget(s) => {
                    let kind = &target::TARGET_TYPE_FILE;
                    let patterns = vec![
                        (parameters::Pattern::Literal(eval::Value::String(
                            Cow::Borrowed(s),
                        ))),
                    ];

                    parser::Decl::resolve_target(
                        ctx,
                        kind,
                        outer,
                        parameters::Patterns(patterns),
                        block,
                    )
                    .add_errors(&mut errors)
                    .unwrap_or_default()
                }
                parser::TargetIdent::Normal { name, pattern } => {
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
                                    ctx,
                                    target_type,
                                    outer,
                                    args,
                                    block,
                                )
                                .add_errors(&mut errors)
                                .unwrap_or_default(),
                            }
                        }
                    }
                }
            },
        };

        errors.ok(target)

        // Target may want to set special parameters, e.g. resolved path

        // Ok(target::Target { kind: self })
    }
}

static TARGET_TYPES: &[target::TargetType<'static>] =
    &[target::TARGET_TYPE_FILE, target::TARGET_TYPE_TASK];

impl<'a> parser::Ast<'a> {
    pub fn resolve(self) -> Result<Decls<'a>, Errors<Error>> {
        let ctx = Ctx {
            functions: eval::BUILTIN_FUNCTIONS,
        };

        let mut errors = Errors::empty();
        let mut decls = Vec::with_capacity(self.decls.len());
        let mut scopes = Vec::new();

        for decl in self.decls {
            decls.push(
                decl.resolve(&ctx, &mut Scopes(&mut scopes), TARGET_TYPES)
                    .add_errors(&mut errors)
                    .unwrap_or_default(),
            );
        }

        errors.ok(decl::Decls(decls))
    }
}
