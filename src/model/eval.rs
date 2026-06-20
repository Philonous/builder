use lasso::Rodeo;
use lasso::RodeoResolver;

use super::command;
use super::parameters;
pub use super::value::{Type, Value};
use crate::error;
use crate::error::WithSpan;
use crate::model::target::{
    self, Target, TargetId, TargetType, TargetTypeId, TargetTypes, Targets,
};
use crate::model::value::display::{DisplayCtx, WithDisplayCtxExt};
use crate::model::value::record;
use crate::model::value::{Ident, Record, RecordSchema, RecordSchemas};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    CommandError(#[from] command::Error),
    #[error("Variable {0} accessed before it was initialised")]
    UninitVariable(String),
    #[error("Variable {0} declared twice in the same block")]
    DoubleDeclare(String),
    #[error("Type error, expected {expected}, found {found}")]
    TypeError { expected: Type, found: Type },
    #[error("{0}")]
    IOError(#[from] std::io::Error),
    #[error("Circular dependency in target {0}")]
    CircularDependency(String),
    #[error("Dependency encountered an error")]
    DependencyError,
    #[error("Target {0} matches multiple patterns")]
    AmbiguousTarget(String),
    #[error("Assertion failed: {0}")]
    AssertionFailed(String),
}

#[derive(Debug, Clone)]
pub struct Var {
    // How many nested contexts up (0 is current)
    pub nesting: usize,
    // Variable slot in the context
    pub slot: usize,
}

#[derive(Clone)]
pub enum FunBody {
    Builtin(fn(&mut EvalContext, &[Deferrable]) -> Result<Value, Error>),
    // TODO: custom functions
}

impl<'a> std::fmt::Debug for FunBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Builtin(_) => write!(f, "<builtin>"),
        }
    }
}

impl FunBody {
    pub fn exec<'b>(&self, args: &[Deferrable], ctx: &mut EvalContext<'b>) -> Result<Value, Error> {
        match self {
            Self::Builtin(f) => f(ctx, args),
        }
    }
}

#[derive(Debug)]
pub struct Fun {
    pub name: Ident,
    // If true, this function cannot commute with external effects So
    // we need to flush outstanding effects (e.g. requested targets
    // that have not yet run)
    pub barrier: bool,
    pub parameters: parameters::Parameters,
    pub body: FunBody,
}

#[derive(Debug, Default)]
pub struct Functions(Vec<Fun>);

impl Functions {
    pub fn find(&self, ident: Ident) -> Option<(FunId, &Fun)> {
        self.0
            .iter()
            .enumerate()
            .find(|(_, f)| f.name == ident)
            .map(|(ix, f)| (FunId(ix), f))
    }
}

impl<'a> std::ops::Index<FunId> for Functions {
    type Output = Fun;

    fn index(&self, index: FunId) -> &Self::Output {
        &self.0[index.0]
    }
}

#[derive(Debug, Clone)]
pub enum StringPart {
    Splice { var: Var, span: error::Span },
    Lit(String),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct FunId(pub usize);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct RecordId(pub usize);

// The actual expression value
#[derive(Debug, Default, Clone)]
pub enum ExprKind {
    Lit(Value),
    String(Vec<StringPart>),
    Var(Var),
    FunApp {
        fun: FunId,
        args: Vec<Expr>,
    },
    Need {
        // Box so we can have a recursive Expr => ExprKind
        request: Box<Expr>,
    },
    Record {
        schema: RecordId,
        fields: Vec<Expr>,
    },
    Command {
        executable: String,
        args: Vec<String>,
    },
    #[default]
    Unit,
}

// Expression with metadata
#[derive(Debug, Clone, Default)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Option<error::Span>,
}

impl<'a> From<Value> for Expr {
    fn from(value: Value) -> Self {
        Self {
            kind: ExprKind::Lit(value),
            span: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Deferrable {
    Deferred {
        target: target::TargetId,
        request: record::Record,
    },
    Immediate(Value),
}

impl From<Value> for Deferrable {
    fn from(value: Value) -> Self {
        Self::Immediate(value)
    }
}

impl Deferrable {
    pub fn force(&self, ctx: &mut EvalContext) -> Result<Value, Error> {
        Ok(match self {
            Deferrable::Deferred { target, request } => {
                ctx.scheduler.force(ctx.resolved, *target, request)?
            }
            Deferrable::Immediate(value) => value.clone(),
        })
    }
}

impl Expr {
    fn eval(&self, ctx: &mut EvalContext) -> Result<Deferrable, error::Spanned<Error>> {
        Ok(match &self.kind {
            ExprKind::Lit(val) => val.clone().into(),
            ExprKind::Var(var) => ctx.read_var(var).with_opt_span(self.span)?.into(),
            ExprKind::Command { executable, args } => {
                // Commands can see effects of targets, so we need to flush
                ctx.flush().without_span()?;
                let cmd = command::Command {
                    executable: executable.clone(),
                    args: args.clone(),
                };
                cmd.exe().with_span_from(self.span)?;
                Value::Unit.into()
            }
            ExprKind::FunApp { fun, args } => {
                let fun = &ctx.resolved[*fun];
                // Functions with observable effects need outstanding
                // targets to be flushed before they can run
                if fun.barrier {
                    ctx.flush().with_opt_span(self.span)?;
                }
                let mut args_acc = Vec::with_capacity(args.len());
                for expr in args.iter() {
                    args_acc.push(expr.eval(ctx)?);
                }

                fun.body
                    .exec(&args_acc, ctx)
                    .with_opt_span(self.span)?
                    .into()
            }
            ExprKind::Need { request } => {
                // TODO: register dependency
                let value = request.eval(ctx)?.force(ctx).with_opt_span(self.span)?;
                let request = value.expect_record(&ctx.resolved).without_span()?;
                match ctx
                    .resolved
                    .targets
                    .match_(ctx.resolved, request)
                    .without_span()?
                {
                    Some(target) => {
                        ctx.scheduler.add(ctx.resolved, request.clone(), target)?;
                        Deferrable::Deferred {
                            target,
                            request: request.clone(),
                        }
                    }
                    None => {
                        todo!()
                        // No target matches requested resource
                        // TODO: register dependency and check existence
                    }
                }
            }
            ExprKind::Record { schema, fields } => {
                let mut values = Vec::with_capacity(fields.len());
                for expr in fields.iter() {
                    // Constructing a record requires concrete values,
                    // we we have t resolve all potentially deferred
                    // values
                    values.push(expr.eval(ctx)?.force(ctx).without_span()?);
                }
                Value::Record(Record {
                    schema: *schema,
                    values,
                })
                .into()
            }
            ExprKind::Unit => Value::Unit.into(),
            ExprKind::String(parts) => {
                let mut s = String::new();
                for part in parts {
                    match part {
                        StringPart::Splice { var, span } => s.push_str(
                            &ctx.read_var(var)
                                .with_span(*span)?
                                .force(ctx)
                                .with_span(*span)?
                                .to_string_with(ctx.resolved),
                        ),
                        StringPart::Lit(cow) => s.push_str(cow),
                    }
                }
                Value::String(s).into()
            }
        })
    }
}

#[derive(Debug)]
pub enum Statement {
    Expr(Expr),
    Let {
        var: Var,
        expr: Expr,
        span: error::Span,
    },
}

impl Statement {
    fn run<'a>(&self, ctx: &mut EvalContext<'a>) -> Result<Deferrable, error::Spanned<Error>> {
        match self {
            Statement::Expr(v) => v.eval(ctx),
            Statement::Let { var, expr, span } => {
                let v = expr.eval(ctx)?;
                ctx.init_var(var, v).with_span(*span)?;
                Ok(Value::Unit.into())
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum SlotState {
    // The analyser should prevent us from reading uninitialised
    // variables, so seeing it is a bug
    Uninitialised,
    /// Needs to be filled in by flushing outstanding deferred
    /// operations, e.g. registered targets
    Value(Deferrable),
}

// Variable Slot
#[derive(Debug, Clone)]
pub struct Slot {
    pub name: Ident,
    pub value: SlotState,
}

#[derive(Debug, Clone, Default)]
pub struct Slots(pub Vec<Slot>);

#[derive(Debug, Default)]
pub struct Block {
    pub statements: Vec<Statement>,
    pub returns_last_value: bool,
    pub slots: Slots,
}

impl<'a> Block {
    pub fn run(&self, ctx: &mut EvalContext<'a>) -> Result<Deferrable, error::Spanned<Error>> {
        // Create new stack frame

        let mut this = ctx.enter(self.slots.clone());

        let mut result = Value::Unit.into();
        for statement in &self.statements {
            let res = statement.run(&mut this)?;
            if self.returns_last_value {
                result = res;
            }
        }
        Ok(result)
    }
}

#[derive(Debug)]
pub struct EvalContext<'a> {
    // TODO: constants
    // Stacked list
    pub variables: &'a mut Vec<Slots>,
    pub resolved: &'a Resolved,
    pub scheduler: &'a mut target::Scheduler,
    //  pub dependencies: &'a mut target::Dependencies,
}

impl<'a> Into<DisplayCtx<'a>> for &'a Resolved {
    fn into(self) -> DisplayCtx<'a> {
        DisplayCtx {
            resolver: &self.resolver,
            records: &self.records,
        }
    }
}

impl<'a> Into<DisplayCtx<'a>> for &'a EvalContext<'a> {
    fn into(self) -> DisplayCtx<'a> {
        self.resolved.into()
    }
}

#[derive(Debug)]
pub struct Resolved {
    pub functions: Functions,
    pub resolver: RodeoResolver,
    pub records: RecordSchemas,
    pub target_types: TargetTypes,
    pub targets: Targets,
}

impl<'a> EvalContext<'a> {
    pub fn enter(&mut self, frame: Slots) -> EvalContext<'_> {
        self.variables.push(frame);
        EvalContext {
            variables: self.variables, // implicit reborrow for the '_ borrow
            resolved: self.resolved,
            scheduler: self.scheduler,
        }
    }

    pub fn flush(&mut self) -> Result<(), Error> {
        self.scheduler.flush(self.resolved)
    }
}

impl<'a> std::ops::Index<FunId> for Resolved {
    type Output = Fun;

    fn index(&self, index: FunId) -> &Self::Output {
        &self.functions[index]
    }
}

impl<'a> std::ops::Index<RecordId> for Resolved {
    type Output = RecordSchema;

    fn index(&self, index: RecordId) -> &Self::Output {
        &self.records[index]
    }
}

impl<'a> std::ops::Index<TargetTypeId> for Resolved {
    type Output = TargetType;

    fn index(&self, index: TargetTypeId) -> &Self::Output {
        &self.target_types[index]
    }
}

impl<'a> std::ops::Index<TargetId> for Resolved {
    type Output = Target;

    fn index(&self, index: TargetId) -> &Self::Output {
        &self.targets[index]
    }
}

impl<'a> std::ops::Index<Ident> for Resolved {
    type Output = str;

    fn index(&self, index: Ident) -> &Self::Output {
        &self.resolver.resolve(&index.0)
    }
}

impl<'a> Drop for EvalContext<'a> {
    fn drop(&mut self) {
        self.variables.pop();
    }
}

impl<'a> EvalContext<'a> {
    fn read_var_raw(&mut self, var: &Var) -> Result<Option<Deferrable>, Error> {
        // The innermost scope is the *last* one
        let ix = &self.variables.len() - var.nesting - 1;
        let slot = &(&self.variables[ix].0)[var.slot];
        match &slot.value {
            // The analyser should not allow this to happen,
            // presuambly this should be a panic instead. TODO: check
            SlotState::Uninitialised => {
                tracing::error!("Uninitialised variable {:?}", var);
                Err(Error::UninitVariable(
                    self.resolved.resolver.resolve(&slot.name.0).to_owned(),
                ))
            }
            SlotState::Value(v) => {
                tracing::trace!("Read variable {:?}, got: {:?}", var, v);
                Ok(Some(v.clone()))
            }
        }
    }

    fn read_var(&mut self, var: &Var) -> Result<Deferrable, Error> {
        Ok(match self.read_var_raw(var)? {
            Some(v) => v,
            // Deferred variable, flush outstanding operations
            // (e.g. requested targets) and try again
            None => {
                self.flush()?;
                self.read_var_raw(var)?
                    .expect("flush did not fill deferred variable")
            }
        })
    }

    fn init_var(&mut self, var: &Var, val: Deferrable) -> Result<(), Error> {
        // The innermost scope is the *last* one
        let ix = self.variables.len() - var.nesting - 1;

        let slot = &mut (&mut self.variables[ix].0)[var.slot];
        match &slot.value {
            SlotState::Uninitialised => {
                slot.value = SlotState::Value(val);
                Ok(())
            }
            // The analyser should not allow this to happen,
            // presuambly this should be a panic instead. TODO: check
            _ => {
                // Can't use self.resolve because self is already borrowed mutably
                let name = self.resolved.resolver.resolve(&slot.name.0);
                Err(Error::DoubleDeclare(name.to_owned()))
            }
        }
    }

    fn assign_var(&mut self, var: &Var, val: Deferrable) -> Result<(), Error> {
        // The innermost scope is the *last* one
        let ix = &self.variables.len() - var.nesting - 1;
        let slot = &mut (&mut self.variables[ix].0)[var.slot];
        match &slot.value {
            SlotState::Uninitialised => {
                // The analyser should not allow this to happen,
                // presuambly this should be a panic instead. TODO: check

                let name = self.resolved.resolver.resolve(&slot.name.0);
                Err(Error::UninitVariable(name.to_owned()))
            }
            // This intentionally overwrites the deferred state. The
            // target will get forced anyway
            _ => {
                slot.value = SlotState::Value(val);
                Ok(())
            }
        }
    }
}

pub fn builtin_fun_print(interner: &mut Rodeo) -> Fun {
    let print = interner.get_or_intern("print");
    let str = interner.get_or_intern("str");
    Fun {
        name: Ident(print),
        parameters: parameters::Parameters(vec![(
            Ident(str),
            parameters::Parameter {
                default: None,
                span: None,
            },
        )]),
        // Print is observable, so we need to flush effects before we
        // can run it
        barrier: true,
        body: FunBody::Builtin(|ctx, args| match args {
            [v] => {
                println!("{}", v.force(ctx)?.with_ctx(ctx.resolved));
                Ok(Value::Unit)
            }
            // Analyser has checked that number of arguments is correct,
            // so this is unreachable
            _ => unreachable!("builtin print: invalid number of arguments"),
        }),
    }
}

// Debug printing that isnt a barrier, so it gets executed without
// flushing outstanding dependencies
pub fn builtin_fun_print_debug(interner: &mut Rodeo) -> Fun {
    let print = interner.get_or_intern("print_debug");
    let str = interner.get_or_intern("str");
    Fun {
        name: Ident(print),
        parameters: parameters::Parameters(vec![(
            Ident(str),
            parameters::Parameter {
                default: None,
                span: None,
            },
        )]),

        // print
        barrier: false,
        body: FunBody::Builtin(|ctx, args| match args {
            [v] => {
                // We *dont* force values here so we can see "under the hood"
                match v {
                    Deferrable::Deferred { request, .. } => {
                        println!("<deferred: {}>", request.with_ctx(ctx.resolved));
                    }
                    Deferrable::Immediate(v) => {
                        println!("{}", v.with_ctx(ctx.resolved));
                    }
                }

                Ok(Value::Unit)
            }
            // Analyser has checked that number of arguments is correct,
            // so this is unreachable
            _ => unreachable!("builtin print: invalid number of arguments"),
        }),
    }
}
// Debug printing that isnt a barrier, so it gets executed without
// flushing outstanding dependencies
pub fn builtin_fun_assert_equal(interner: &mut Rodeo) -> Fun {
    let print = interner.get_or_intern("assert_equal");
    let left = interner.get_or_intern("left");
    let right = interner.get_or_intern("right");
    Fun {
        name: Ident(print),
        parameters: parameters::Parameters(vec![
            (
                Ident(left),
                parameters::Parameter {
                    default: None,
                    span: None,
                },
            ),
            (
                Ident(right),
                parameters::Parameter {
                    default: None,
                    span: None,
                },
            ),
        ]),

        // print
        barrier: false,
        body: FunBody::Builtin(|ctx, args| match args {
            [left, right] => {
                let left = left.force(ctx)?;
                let right = right.force(ctx)?;
                if (left != right) {
                    return Err(Error::AssertionFailed(format!(
                        "{} != {}",
                        left.with_ctx(&*ctx),
                        right.with_ctx(&*ctx)
                    )));
                }
                Ok(Value::Unit)
            }
            // Analyser has checked that number of arguments is correct,
            // so this is unreachable
            _ => unreachable!("builtin print: invalid number of arguments"),
        }),
    }
}

pub fn builtin_functions(interner: &mut Rodeo) -> Functions {
    Functions(vec![
        builtin_fun_print(interner),
        builtin_fun_print_debug(interner),
        builtin_fun_assert_equal(interner),
    ])
}
