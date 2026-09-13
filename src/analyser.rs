use std::path::Path;

use lasso::{Interner, Resolver, Rodeo};

use super::model::{eval, parameters, target};
use super::parser::parser;
use crate::error::{self, AddErrors, Errors, Spanned, WithSpan};
use crate::model::eval::{Functions, SlotState, Slots};
use crate::model::target::{
    ModuleId, TargetType, TargetTypeId, TargetTypes, Targets,
};
use crate::model::value::{Ident, RecordPattern, RecordSchemas};
use crate::parser::string::QuoteError;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("variable {0} was already declared")]
    DuplicateVariable(String),
    #[error("variable {0} not declared")]
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
    #[error("{0}")]
    TargetError(#[from] target::Error),
}

impl<'a> parser::Ident<'a> {
    pub fn resolve(self, interner: &mut impl Interner) -> Ident {
        Ident(interner.get_or_intern(self.0))
    }
}

impl Slots {
    // Create a new variable in the current scope
    fn new_variable(
        &mut self,
        ctx: &Ctx,
        name: Ident,
    ) -> Result<eval::Var, Error> {
        // Check if the variable is already declared
        match self.find_variable(name) {
            Some(_) => {
                return Err(Error::DuplicateVariable(
                    ctx.resolve(name).to_string(),
                ));
            }
            None => (),
        }
        // Create a new slot for the variable
        self.0.push(eval::Slot {
            name: name.clone(),
            value: SlotState::Uninitialised,
        });

        // Create variable referencing slot
        Ok(eval::Var {
            // New variables are always created in the *current*
            // scope, so nesting is 0
            nesting: 0,
            slot: self.0.len() - 1,
        })
    }

    fn find_variable(&self, name: Ident) -> Option<usize> {
        self.0.iter().position(|slot| slot.name == name)
    }
}

struct Scopes<'b>(&'b mut Vec<Slots>);

impl<'b> Scopes<'b> {
    /// Push a fresh scope; the returned guard pops it when it leaves.
    fn enter(&mut self) -> ScopeGuard<'_> {
        self.0.push(Slots(Vec::new()));
        ScopeGuard(Scopes(&mut *self.0)) // reborrow, so the guard can't outlive us
    }
}

struct ScopeGuard<'b>(Scopes<'b>);

impl<'b> ScopeGuard<'b> {
    fn new_variable(
        &mut self,
        ctx: &Ctx,
        name: Ident,
    ) -> Result<eval::Var, Error> {
        // Last element is the current scope where we want to create
        // the new variable

        self.0
             .0
            .last_mut()
            .expect("ScopeGuard: empty")
            .new_variable(ctx, name)
    }
    fn find_variable(
        &self,
        ctx: &mut Ctx,
        name: Ident,
    ) -> Result<eval::Var, Error> {
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
            .ok_or_else(|| {
                Error::UndefinedVariable(ctx.resolve(name).to_string())
            })
    }

    /// Success path: pop and take ownership of the scope (e.g. to attach to a block).
    fn detach(mut self) -> Slots {
        let Scopes(vec) = &mut self.0;
        let slots = vec.pop().expect("guard guarantees a live scope");
        std::mem::forget(self); // skip Drop so we don't pop twice
        slots
    }

    /// Add a new nested scope
    fn nested_scope(&mut self) -> &mut Scopes<'b> {
        &mut self.0
    }
}

impl<'b> Drop for ScopeGuard<'b> {
    fn drop(&mut self) {
        let Scopes(vec) = &mut self.0;
        vec.pop(); // error/early-return path
    }
}

impl<'a> parser::Lit<'a> {
    pub fn resolve<'b>(self) -> Result<eval::Value, Error> {
        Ok(match self {
            parser::Lit::String(s) => eval::Value::String(s.unquote()?),
            parser::Lit::Bool(b) => eval::Value::Bool(b),
        })
    }
}

pub struct Ctx<'fs> {
    pub interner: &'fs mut Rodeo,
    pub functions: &'fs Functions,
    pub records: &'fs RecordSchemas,
    pub target_types: &'fs TargetTypes,
    pub module: ModuleId,
    // todo: constants
}

impl<'fs> Ctx<'fs> {
    pub fn intern<'a>(&mut self, i: parser::Ident<'a>) -> Ident {
        Ident(self.interner.get_or_intern(i.0))
    }

    pub fn resolve<'a>(&'a self, i: Ident) -> &'a str {
        self.interner.resolve(&i.0)
    }
}

impl<'a> parser::StringExpr<'a> {
    fn resolve<'b>(
        self,
        ctx: &'a mut Ctx,
        scopes: &mut ScopeGuard<'b>,
    ) -> Result<eval::ExprKind, Errors<Error>> {
        let mut errors = Errors::empty();
        let mut parts = Vec::with_capacity(self.0.len());
        for part in self.0 {
            match part {
                parser::StringExprPart::Lit(quoted) => {
                    let _: Option<_> = quoted
                        .inner
                        .unquote()
                        .with_span(quoted.span)
                        .add_errors(&mut errors)
                        .map(|s| parts.push(eval::StringPart::Lit(s)));
                }
                parser::StringExprPart::Splice(ast_ident) => {
                    let ident = ctx.intern(ast_ident.inner);
                    scopes
                        .find_variable(ctx, ident)
                        .with_span(ast_ident.span)
                        .add_errors(&mut errors)
                        .map(|s| {
                            parts.push(eval::StringPart::Splice {
                                var: s,
                                span: ast_ident.span.into(),
                            })
                        });
                }
            }
        }
        errors.ok(eval::ExprKind::String(parts))
    }
}

impl<'a> parser::CommandPart<'a> {
    fn resolve<'b>(
        self,
        ctx: &'a mut Ctx,
        scopes: &mut ScopeGuard<'b>,
    ) -> Result<eval::Expr, Errors<Error>> {
        let mut errors = Errors::empty();
        let part = match self {
            parser::CommandPart::BareString(s) => eval::Expr {
                kind: eval::ExprKind::String(vec![eval::StringPart::Lit(
                    s.inner.to_owned(),
                )]),
                span: Some(s.span.into()),
            },
            parser::CommandPart::Splice(ident) => eval::Expr {
                span: Some(ident.span.into()),
                kind: eval::ExprKind::Var({
                    let var = ctx.intern(ident.inner);
                    scopes
                        .find_variable(ctx, var)
                        .with_span(ident.span)
                        .add_errors(&mut errors)
                        .unwrap_or_default()
                }),
            },
            parser::CommandPart::String(string_expr) => eval::Expr {
                kind: string_expr
                    .resolve(ctx, scopes)
                    .add_errors(&mut errors)
                    .unwrap_or_default(),
                span: None,
            },
        };
        errors.ok(part)
    }
}

impl<'a> parser::Expr<'a> {
    fn resolve<'b>(
        self,
        ctx: &'a mut Ctx,
        scopes: &mut ScopeGuard<'b>,
    ) -> Result<eval::Expr, Errors<Error>> {
        let mut errors = Errors::empty();
        let span = self.0.span.into();
        let kind = match self.0.inner {
            parser::ExprKind::Lit(lit) => eval::ExprKind::Lit(
                lit.resolve()
                    .with_span(span)
                    .add_errors(&mut errors)
                    .unwrap_or_default(),
            ),

            parser::ExprKind::Ident(name) => {
                let ident = name.resolve(&mut ctx.interner);
                scopes
                    .find_variable(ctx, ident)
                    .map(eval::ExprKind::Var)
                    .with_span(span)
                    .add_errors(&mut errors)
                    .unwrap_or_default()
            }
            parser::ExprKind::Command(parser::Command { executable, args }) => {
                eval::ExprKind::Command {
                    executable: Box::new(
                        executable
                            .resolve(ctx, scopes)
                            .add_errors(&mut errors)
                            .unwrap_or_default(),
                    ),
                    args: args
                        .into_iter()
                        .map(|part| {
                            part.resolve(ctx, scopes)
                                .add_errors(&mut errors)
                                .unwrap_or_default()
                        })
                        .collect(),
                }
            }
            parser::ExprKind::FunApp(ast_ident, args) => (|| {
                // Dealing with things that look like function
                // applications: `foo(bar, quux=3)`, which can either
                // be a function proper or a record constructor
                let ident = ctx.intern(ast_ident);
                // Check if it's a function
                if let Some((ix, fun)) = ctx.functions.find(ident) {
                    let args_separate = args
                        .inner
                        .resolve(ctx, scopes)
                        .add_errors(&mut errors)?;

                    let args = fun
                        .parameters
                        .match_arguments(
                            &ctx.interner,
                            error::Spanned {
                                inner: &args_separate,
                                span: Some(args.span.into()),
                            },
                        )
                        .add_errors(&mut errors)?;
                    Some(eval::ExprKind::FunApp { fun: ix, args })
                // Check if it's a record
                } else if let Some((ix, record)) = ctx.records.find(ident) {
                    let args_separate = args
                        .inner
                        .resolve(ctx, scopes)
                        .add_errors(&mut errors)?;

                    let fields = record
                        .fields
                        .match_arguments(
                            &ctx.interner,
                            error::Spanned {
                                inner: &args_separate,
                                span: Some(args.span.into()),
                            },
                        )
                        .add_errors(&mut errors)?;
                    Some(eval::ExprKind::Record { schema: ix, fields })
                } else {
                    Err(Spanned::spanned(
                        Error::FunNotFound(ast_ident.to_string()),
                        span,
                    ))
                    .add_errors(&mut errors)?
                }
            })()
            .unwrap_or_default(),
            parser::ExprKind::String(string_expr) => string_expr
                .resolve(ctx, scopes)
                .add_errors(&mut errors)
                .unwrap_or_default(),
            parser::ExprKind::Need(parser::NeedExpr(request)) => {
                eval::ExprKind::Need {
                    request: Box::new(
                        request
                            .resolve(ctx, scopes)
                            .add_errors(&mut errors)
                            .unwrap_or_default(),
                    ),
                }
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
        ctx: &'a mut Ctx,
        scopes: &mut ScopeGuard<'b>,
    ) -> Result<parameters::Arguments<eval::Expr>, Errors<Error>> {
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
                    let ident = ctx.intern(name);
                    expr.map(|expr| {
                        key_value
                            .push(Spanned::spanned((ident, expr), arg.span))
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
    pub fn resolve(
        self,
        ctx: &mut Ctx,
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
                    let ident = var.resolve(&mut ctx.interner);
                    key_value.push(Spanned::spanned(
                        (ident, parameters::Pattern::Bind(ident)),
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
                        parser::PatternKind::Bind(_) => {
                            unreachable!(
                                "bare variable pattern already handled"
                            )
                        }
                        parser::PatternKind::Wildcard => {
                            parameters::Pattern::Wildcard
                        }
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
                            parameters::Pattern::Bind(
                                var.resolve(&mut ctx.interner),
                            )
                        }
                        parser::PatternKind::Wildcard => {
                            parameters::Pattern::Wildcard
                        }
                    };
                    key_value.push(Spanned::spanned(
                        (name.resolve(&mut ctx.interner), res),
                        span,
                    ));
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
        ctx: &mut Ctx<'_>,
        outer: &mut Scopes<'scopes>,
    ) -> Result<eval::Block, Errors<Error>> {
        let mut errors = Errors::empty();
        let mut this = outer.enter();
        let mut resolved: Vec<eval::Statement> =
            Vec::with_capacity(self.statements.len());
        for statement in self.statements {
            let stmt = (|| {
                match statement {
                    parser::Statement::Let(ident, expr) => {
                        // let var = expr
                        // Resolve expression first so it sees outer scope
                        // Ordering matters here, we don't want the
                        // expression to see the variable it initialises
                        let expr = expr
                            .resolve(ctx, &mut this)
                            .add_errors(&mut errors)?;
                        // Now create variable in inner scope
                        let name = ident.resolve(&mut ctx.interner);
                        let var = this
                            .new_variable(ctx, name)
                            .with_span(ident.span)
                            .add_errors(&mut errors)?;
                        // Create resolved statement if both var and expr are `Some`
                        Some(eval::Statement::Let {
                            var,
                            span: ident.span.into(),
                            expr,
                        })
                    }
                    parser::Statement::Expr(expr) => {
                        Some(eval::Statement::Expr(
                            expr.resolve(ctx, &mut this)
                                .add_errors(&mut errors)?,
                        ))
                    }
                }
            })();
            // Add statement if it resolved, if any return an error,
            // we don't return the list at all, so we never return a
            // partia list
            stmt.map(|stmt| resolved.push(stmt));
        }

        let empty_slots = this.detach();
        errors.ok(eval::Block {
            statements: resolved,
            returns_last_value: self.returns_last_value,
            slots: empty_slots,
        })
    }
}

impl<'a> parser::Decl<'a> {
    fn resolve_target<'b>(
        ctx: &mut Ctx<'_>,
        kind: &TargetType,
        type_id: TargetTypeId,
        outer: &mut Scopes<'b>,
        parameters: parameters::Patterns,
        block: parser::Block<'a>,
    ) -> Result<target::Target, Errors<Error>> {
        // target Constr(patter1, pattern2, ...)

        let mut this_scope = outer.enter();
        let mut errors: Errors<Error> = Errors::empty();

        // Find the "name" field that determines if the target is
        // displayed in the CLI
        let name = match &parameters.0[kind.name_is] {
            parameters::Pattern::Literal(eval::Value::String(s)) => {
                Some(s.clone())
            }
            _ => None,
        };

        let mut matchers = Vec::new();
        for (ix, pat) in parameters.0.iter().enumerate() {
            match pat {
                parameters::Pattern::Literal { .. }
                | parameters::Pattern::Wildcard => (),
                parameters::Pattern::Bind(name) => {
                    this_scope
                        .new_variable(ctx, *name)
                        .without_span()
                        .add_errors(&mut errors);
                    matchers.push(ix);
                }
            }
        }

        // Resolve the command block
        let block = block
            .resolve(ctx, this_scope.nested_scope())
            .add_errors(&mut errors)
            .unwrap_or_default();

        let slots = this_scope.detach();

        let pattern = RecordPattern {
            schema: kind.request_schema,
            patterns: parameters,
        };
        let target = target::Target {
            kind: type_id,
            name,
            pattern,
            block,
            slots,
            matchers,
            module: ctx.module,
        };

        errors.ok(target)
    }

    fn resolve<'scopes>(
        self,
        ctx: &mut Ctx<'_>,
        outer: &mut Scopes<'scopes>,
    ) -> Result<target::Target, Errors<Error>> {
        let mut errors: Errors<Error> = Errors::empty();

        let target = match self {
            parser::Decl::Target { args, ident, block } => match ident {
                parser::TargetIdent::AtTarget(s) => {
                    let name = Ident(ctx.interner.get_or_intern_static("file"));
                    let (schema, _) =
                        ctx.records.find(name).expect("file record schema");
                    let (type_id, target_type) = ctx
                        .target_types
                        .find_by_request(schema)
                        .expect("file target type");
                    let patterns = vec![
                        (parameters::Pattern::Literal(eval::Value::String(
                            String::from(s),
                        ))),
                    ];

                    parser::Decl::resolve_target(
                        ctx,
                        target_type,
                        type_id,
                        outer,
                        parameters::Patterns(patterns),
                        block,
                    )
                    .add_errors(&mut errors)
                    .unwrap_or_default()
                }
                parser::TargetIdent::Normal {
                    name: ast_name,
                    pattern,
                } => {
                    let name = ctx.intern(ast_name.inner);
                    let target_type = (|| {
                        let (schema_id, schema) = ctx.records.find(name)?;
                        let (type_id, target_type) =
                            ctx.target_types.find_by_request(schema_id)?;
                        Some((target_type, type_id, schema))
                    })()
                    .ok_or(Error::UnknownTargetType(ast_name.inner.to_string()))
                    .with_span(ast_name.span)
                    .add_errors(&mut errors);

                    // Resolve pattern
                    let pattern = pattern.resolve(ctx).add_errors(&mut errors);

                    match target_type.zip(pattern) {
                        // Previous error, skip checking pattern
                        None => {
                            // Errors must already have been set, so
                            // we simply return a dummy variable
                            Default::default()
                        }

                        // Match patterns to parameters
                        Some(((target_type, type_id, schema), pattern)) => {
                            let pattern = schema
                                .fields
                                .match_pattern_arguments(
                                    &ctx.interner,
                                    &pattern,
                                )
                                .add_errors(&mut errors);
                            match pattern {
                                None => Default::default(),
                                // We have collected all the data, now
                                // we can resolve the targets block
                                Some(args) => parser::Decl::resolve_target(
                                    ctx,
                                    target_type,
                                    type_id,
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
    }
}

fn register_builtin_target_types(
    interner: &mut Rodeo,
    records: &mut RecordSchemas,
    target_types: &mut TargetTypes,
) -> Result<(), Error> {
    target::register_target_type_file(interner, records, target_types)?;
    target::register_target_type_task(interner, records, target_types)?;
    Ok(())
}

impl<'a> parser::Ast<'a> {
    pub fn resolve(
        self,
        module: ModuleId,
    ) -> Result<eval::Resolved<Rodeo>, Errors<Error>> {
        let mut errors = Errors::empty();

        let mut interner = Rodeo::new();
        let functions = eval::builtin_functions(&mut interner);
        let mut target_types = Default::default();
        let mut records = Default::default();

        register_builtin_target_types(
            &mut interner,
            &mut records,
            &mut target_types,
        )
        .expect("register builtins");

        let mut targets: Targets = Default::default();
        let mut scopes = Vec::new();

        let mut ctx = Ctx {
            interner: &mut interner,
            functions: &functions,
            records: &records,
            target_types: &target_types,
            module,
        };
        for decl in self.decls {
            let t = decl
                .resolve(&mut ctx, &mut Scopes(&mut scopes))
                .add_errors(&mut errors)
                .unwrap_or_default();
            targets
                .add(t)
                .without_span()
                .add_errors(&mut errors)
                .unwrap_or_default();
        }

        errors.ok(eval::Resolved {
            functions,
            resolver: interner,
            records,
            target_types,
            targets,
            modules: target::Modules::new(),
        })
    }
}
