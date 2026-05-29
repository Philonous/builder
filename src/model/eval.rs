use std::borrow::Cow;

pub use super::base::{Ident, Value};
use super::command;
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

#[derive(Debug)]
pub struct Var {
    // How many nested contexts up (0 is current)
    pub nesting: usize,
    // Variable slot in the context
    pub slot: usize,
}

// The actual expression value
#[derive(Debug)]
pub enum ExprKind<'a> {
    Lit(Value<'a>),
    Var(Var),
    // FunApp(Ident<'a>),
    Command {
        executable: Cow<'a, str>,
        args: Vec<Cow<'a, str>>,
    },
}

// Expression with metadata
#[derive(Debug)]
pub struct Expr<'a> {
    pub kind: ExprKind<'a>,
    pub span: error::Span,
}

impl<'a> Expr<'a> {
    fn eval<'outer>(
        &self,
        ctx: &mut EvalContext<'outer, 'a>,
    ) -> Result<Value<'a>, error::Spanned<Error>> {
        Ok(match &self.kind {
            ExprKind::Lit(val) => val.clone(),
            ExprKind::Var(var) => ctx.read_var(var).with_span(self.span)?,
            // Expr::FunApp(ident) => todo!(),
            ExprKind::Command { executable, args } => {
                let cmd = command::Command {
                    executable: executable.clone(),
                    args: args.clone(),
                };
                cmd.exe().with_span_from(self.span)?;
                Value::Unit
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
    pub empty_slots: Slots<'a>,
}

impl<'a> Block<'a> {
    fn run<'outer>(
        &self,
        ctx: &mut EvalContext<'outer, 'a>,
    ) -> Result<Value<'a>, error::Spanned<Error>> {
        // Create new stack frame
        let this_slots = self.empty_slots.clone();
        let mut this = EvalContext {
            variables: ctx.variables,
        };

        let mut result = Value::Unit;
        for statement in &self.statements {
            result = statement.run(&mut this)?;
        }
        Ok(result)
    }
}

#[derive(Debug)]
struct EvalContext<'outer, 'a> {
    variables: &'outer mut Vec<Slots<'a>>,
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
        let ix = &self.variables.len() - var.nesting - 1;
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

// impl <'a> Block<'a> {
//     eval
// }
