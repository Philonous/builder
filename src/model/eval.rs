use std::borrow::Cow;

pub use super::base::{Ident, Value};
use super::command;
use super::parameters;
use crate::error;
use crate::error::WithSpan;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    CommandError(#[from] command::Error),
    #[error("Variable {0} accessed before it was initialised")]
    UninitVariable(String),
    #[error("Variable {0} declared twice in the same block")]
    DoubleDeclare(String),
}

#[derive(Debug, Clone)]
pub struct Var {
    // How many nested contexts up (0 is current)
    pub nesting: usize,
    // Variable slot in the context
    pub slot: usize,
}

pub enum FunBody {
    Builtin(for<'b> fn(&[Value<'b>]) -> Result<Value<'b>, Error>),
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
    pub fn exec<'outer, 'b>(
        &self,
        args: &[Value<'b>],
        _ctx: &mut EvalContext<'outer, 'b>,
    ) -> Result<Value<'b>, Error> {
        match self {
            Self::Builtin(f) => f(args),
        }
    }
}

#[derive(Debug)]
pub struct Fun<'a> {
    pub name: Ident<'a>,
    pub parameters: parameters::Parameters<'a>,
    pub body: FunBody,
}

#[derive(Debug, Clone)]
pub enum StringPart<'a> {
    Splice { var: Var, span: error::Span },
    Lit(Cow<'a, str>),
}

// The actual expression value
#[derive(Debug, Default, Clone)]
pub enum ExprKind<'a> {
    Lit(Value<'a>),
    String(Vec<StringPart<'a>>),
    Var(Var),
    FunApp {
        fun: &'a Fun<'a>,
        args: Vec<Expr<'a>>,
    },
    Command {
        executable: Cow<'a, str>,
        args: Vec<Cow<'a, str>>,
    },
    #[default]
    Unit,
}

// Expression with metadata
#[derive(Debug, Clone)]
pub struct Expr<'a> {
    pub kind: ExprKind<'a>,
    pub span: Option<error::Span>,
}

impl<'a> From<Value<'a>> for Expr<'a> {
    fn from(value: Value<'a>) -> Self {
        Self {
            kind: ExprKind::Lit(value),
            span: None,
        }
    }
}

impl<'a> Expr<'a> {
    fn eval<'outer>(
        &self,
        ctx: &mut EvalContext<'outer, 'a>,
    ) -> Result<Value<'a>, error::Spanned<Error>> {
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
                fun.body.exec(&args_acc, ctx).with_opt_span(self.span)?
            }
            ExprKind::Unit => Value::Unit,
            ExprKind::String(parts) => {
                let mut s = String::new();
                for part in parts {
                    match part {
                        StringPart::Splice { var, span } => s.push_str(
                            &ctx.read_var(var).with_span(*span)?.to_string(),
                        ),
                        StringPart::Lit(cow) => s.push_str(cow),
                    }
                }
                Value::String(Cow::Owned(s))
            }
        })
    }
}

#[derive(Debug)]
pub enum Statement<'a> {
    Expr(Expr<'a>),
    Let {
        var: Var,
        expr: Expr<'a>,
        span: error::Span,
    },
}

impl<'a> Statement<'a> {
    fn run<'outer>(
        &self,
        ctx: &mut EvalContext<'outer, 'a>,
    ) -> Result<Value<'a>, error::Spanned<Error>> {
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
pub struct Slot<'a> {
    pub name: Ident<'a>,
    pub value: Option<Value<'a>>,
}

#[derive(Debug, Clone, Default)]
pub struct Slots<'a>(pub Vec<Slot<'a>>);

#[derive(Debug, Default)]
pub struct Block<'a> {
    pub statements: Vec<Statement<'a>>,
    pub slots: Slots<'a>,
}

impl<'a> Block<'a> {
    pub fn run<'outer>(
        &self,
        ctx: &mut EvalContext<'outer, 'a>,
    ) -> Result<Value<'a>, error::Spanned<Error>> {
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
pub struct EvalContext<'outer, 'a> {
    // TODO: constants
    pub variables: &'outer mut Vec<Slots<'a>>,
}

impl<'outer, 'a> EvalContext<'outer, 'a> {
    fn enter(&mut self, frame: Slots<'a>) -> EvalContext<'_, 'a> {
        self.variables.push(frame);
        EvalContext {
            variables: self.variables, // implicit reborrow for the '_ borrow
        }
    }
}

impl<'outer, 'a> Drop for EvalContext<'outer, 'a> {
    fn drop(&mut self) {
        self.variables.pop();
    }
}

impl<'outer, 'a> EvalContext<'outer, 'a> {
    fn read_var(&self, var: &Var) -> Result<Value<'a>, Error> {
        // The innermost scope is the *last* one
        let ix = &self.variables.len() - var.nesting - 1;
        let slot = &(&self.variables[ix].0)[var.slot];
        match &slot.value {
            // Analyser should catch this before we hit it here
            None => Err(Error::UninitVariable(slot.name.to_string())),
            Some(v) => Ok(v.clone()),
        }
    }

    fn init_var(&mut self, var: &Var, val: Value<'a>) -> Result<(), Error> {
        // The innermost scope is the *last* one
        eprintln!(
            "init_var : len: {}, nesting: {}",
            self.variables.len(),
            var.nesting
        );
        let ix = self.variables.len() - var.nesting - 1;
        let slot = &mut (&mut self.variables[ix].0)[var.slot];
        match &slot.value {
            None => {
                slot.value = Some(val);
                Ok(())
            }
            Some(_) => Err(Error::DoubleDeclare(slot.name.to_string())),
        }
    }

    fn assign_var(&mut self, var: &Var, val: Value<'a>) -> Result<(), Error> {
        // The innermost scope is the *last* one
        let ix = &self.variables.len() - var.nesting - 1;
        let slot = &mut (&mut self.variables[ix].0)[var.slot];
        match &slot.value {
            None => Err(Error::UninitVariable(slot.name.to_string())),
            Some(_) => {
                slot.value = Some(val);
                Ok(())
            }
        }
    }
}

const BUILT_FUN_PRINT: Fun<'static> = Fun {
    name: Ident("print"),
    parameters: parameters::Parameters(parameters::CowSlice::Borrowed(&[(
        Ident("str"),
        parameters::Parameter {
            default: None,
            span: None,
        },
    )])),
    body: FunBody::Builtin(|args| match args {
        [str] => {
            println!("{}", str);
            Ok(Value::Unit)
        }
        // Analyser has checked that number of arguments is correct,
        // so this is unreachable
        _ => unreachable!("builtin print: invalid number of arguments"),
    }),
};

pub const BUILTIN_FUNCTIONS: &'static [Fun<'static>] = &[BUILT_FUN_PRINT];
