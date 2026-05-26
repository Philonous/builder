use super::model::eval;
use super::parser::parser;
use crate::error::WithSpan;
use crate::{
    common::Ident,
    error::{self, Spanned},
    model::eval::Slots,
    parser::string::QuoteError,
};

use std::ops::{Deref, DerefMut};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Variable {0} was already declared")]
    DuplicateVariable(String),
    #[error("Variable {0} used before being declared")]
    UndefinedVariable(String),
    #[error("{0}")]
    QuoteError(#[from] QuoteError),
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

    fn borrowed(&mut self) -> &mut Scopes<'a, 'b> {
        &mut self.0
    }
}

impl<'a, 'b> Drop for ScopeGuard<'a, 'b> {
    fn drop(&mut self) {
        let Scopes(vec) = &mut self.0;
        vec.pop(); // error/early-return path
    }
}

// impl<'a> Scopes<'a> {}

impl<'a> parser::Expr<'a> {
    pub fn resolve<'b>(
        self,
        scopes: &mut ScopeGuard<'a, 'b>,
    ) -> Result<eval::Expr<'a>, Spanned<Error>> {
        let span = self.0.span.into();
        let kind = match self.0.inner {
            parser::ExprKind::Lit(lit) => match lit {
                parser::Lit::String(s) => eval::ExprKind::Lit(
                    eval::Value::String(s.unquote().with_span(span)?),
                ),
            },

            parser::ExprKind::Ident(name) => scopes
                .find_variable(&name)
                .map(eval::ExprKind::Var)
                .with_span(span)?,
            parser::ExprKind::Command(cmd) => cmd
                .resolve_strings()
                .map(|parser::Command { executable, args }| {
                    eval::ExprKind::Command { executable, args }
                })
                .with_span(span)?,
            parser::ExprKind::FunApp(ident, exprs) => unimplemented!(),
        };
        Ok(eval::Expr { kind, span })
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

// impl<'a> parser::Decl<'a> {
//     pub fn resolve<'scopes>(
//         self,
//         outer: &mut Scopes<'a, 'scopes>,
//     ) -> Result<eval::Decl<'a>, Spanned<Error>> {
//     }
// }
