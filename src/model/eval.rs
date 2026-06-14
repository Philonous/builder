use lasso::Rodeo;
use lasso::RodeoResolver;

use super::command;
use super::parameters;
pub use super::value::{Type, Value};
use crate::error;
use crate::error::WithSpan;
use crate::model::target::TargetType;
use crate::model::target::TargetTypeId;
use crate::model::target::{TargetTypes, Targets};
use crate::model::value::display::{DisplayCtx, WithDisplayCtxExt};
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
    Builtin(fn(&mut EvalContext, &[Value]) -> Result<Value, Error>),
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
    pub fn exec<'b>(
        &self,
        args: &[Value],
        ctx: &mut EvalContext<'b>,
    ) -> Result<Value, Error> {
        match self {
            Self::Builtin(f) => f(ctx, args),
        }
    }
}

#[derive(Debug)]
pub struct Fun {
    pub name: Ident,
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
#[derive(Debug, Clone)]
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

impl Expr {
    fn eval(
        &self,
        ctx: &mut EvalContext,
    ) -> Result<Value, error::Spanned<Error>> {
        Ok(match &self.kind {
            ExprKind::Lit(val) => val.clone(),
            ExprKind::Var(var) => ctx.read_var(var).with_opt_span(self.span)?,
            // Expr::FunApp(ident) => todo!(),
            ExprKind::Command { executable, args } => {
                let cmd = command::Command {
                    executable: executable.clone(),
                    args: args.clone(),
                };
                cmd.exe().with_span_from(self.span)?;
                Value::Unit
            }
            ExprKind::FunApp { fun, args } => {
                let mut args_acc = Vec::with_capacity(args.len());
                for expr in args.iter() {
                    args_acc.push(expr.eval(ctx)?);
                }
                let fun = &ctx.resolved[*fun];
                fun.body.exec(&args_acc, ctx).with_opt_span(self.span)?
            }
            ExprKind::Record { schema, fields } => {
                let mut values = Vec::with_capacity(fields.len());
                for expr in fields.iter() {
                    values.push(expr.eval(ctx)?);
                }
                Value::Record(Record {
                    schema: *schema,
                    values,
                })
            }
            ExprKind::Unit => Value::Unit,
            ExprKind::String(parts) => {
                let mut s = String::new();
                for part in parts {
                    match part {
                        StringPart::Splice { var, span } => s.push_str(
                            &ctx.read_var(var)
                                .with_span(*span)?
                                .to_string_with(ctx.resolved),
                        ),
                        StringPart::Lit(cow) => s.push_str(cow),
                    }
                }
                Value::String(s)
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
    fn run<'a>(
        &self,
        ctx: &mut EvalContext<'a>,
    ) -> Result<Value, error::Spanned<Error>> {
        match self {
            Statement::Expr(v) => v.eval(ctx),
            Statement::Let { var, expr, span } => {
                let v = expr.eval(ctx)?;
                ctx.init_var(var, v).with_span(*span)?;
                Ok(Value::Unit)
            }
        }
    }
}

// Variable Slot
#[derive(Debug, Clone)]
pub struct Slot {
    pub name: Ident,
    pub value: Option<Value>,
}

#[derive(Debug, Clone, Default)]
pub struct Slots(pub Vec<Slot>);

#[derive(Debug, Default)]
pub struct Block {
    pub statements: Vec<Statement>,
    pub slots: Slots,
}

impl<'a> Block {
    pub fn run(
        &self,
        ctx: &mut EvalContext<'a>,
    ) -> Result<Value, error::Spanned<Error>> {
        // Create new stack frame

        let mut this = ctx.enter(self.slots.clone());

        let mut result = Value::Unit;
        for statement in &self.statements {
            result = statement.run(&mut this)?;
        }
        Ok(result)
    }
}

#[derive(Debug)]
pub struct EvalContext<'a> {
    // TODO: constants
    pub variables: &'a mut Vec<Slots>,
    pub resolved: &'a Resolved,
}

impl<'a> Into<DisplayCtx<'a>> for &'a Resolved {
    fn into(self) -> DisplayCtx<'a> {
        DisplayCtx {
            resolver: &self.resolver,
            records: &self.records,
        }
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
        }
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
    fn read_var(&self, var: &Var) -> Result<Value, Error> {
        // The innermost scope is the *last* one
        let ix = &self.variables.len() - var.nesting - 1;
        let slot = &(&self.variables[ix].0)[var.slot];
        match &slot.value {
            // Analyser should catch this before we hit it here
            None => Err(Error::UninitVariable(
                self.resolved.resolver.resolve(&slot.name.0).to_owned(),
            )),
            Some(v) => Ok(v.clone()),
        }
    }

    fn init_var(&mut self, var: &Var, val: Value) -> Result<(), Error> {
        // The innermost scope is the *last* one
        let ix = self.variables.len() - var.nesting - 1;

        let slot = &mut (&mut self.variables[ix].0)[var.slot];
        match &slot.value {
            None => {
                slot.value = Some(val);
                Ok(())
            }
            Some(_) => {
                // Can't use self.resolve because self is already borrowed mutably
                let name = self.resolved.resolver.resolve(&slot.name.0);
                Err(Error::DoubleDeclare(name.to_owned()))
            }
        }
    }

    fn assign_var(&mut self, var: &Var, val: Value) -> Result<(), Error> {
        // The innermost scope is the *last* one
        let ix = &self.variables.len() - var.nesting - 1;
        let slot = &mut (&mut self.variables[ix].0)[var.slot];
        match &slot.value {
            None => {
                let name = self.resolved.resolver.resolve(&slot.name.0);
                Err(Error::UninitVariable(name.to_owned()))
            }
            Some(_) => {
                slot.value = Some(val);
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
        body: FunBody::Builtin(|ctx, args| match args {
            [v] => {
                println!("{}", v.with_ctx(ctx.resolved));
                Ok(Value::Unit)
            }
            // Analyser has checked that number of arguments is correct,
            // so this is unreachable
            _ => unreachable!("builtin print: invalid number of arguments"),
        }),
    }
}

pub fn builtin_functions(interner: &mut Rodeo) -> Functions {
    Functions(vec![builtin_fun_print(interner)])
}
