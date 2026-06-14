pub mod display;
pub mod record;

use std::fmt;

use super::eval::{self, Resolved};
pub use display::{DisplayCtx, DisplayWith, WithDisplayCtxExt};
use lasso::Spur;
pub use record::{Record, RecordPattern, RecordSchema, RecordSchemas};

#[derive(Eq, Hash, PartialEq, Debug, Clone, Copy)]
pub struct Ident(pub Spur);

impl Default for Ident {
    fn default() -> Self {
        Self(Default::default())
    }
}

impl DisplayWith for Ident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>, ctx: DisplayCtx) -> fmt::Result {
        write!(f, "{}", ctx.resolver.resolve(&self.0))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Type {
    Unit,
    String,
    Record(String),
    Int,
}

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Unit => write!(f, "()"),
            Type::String => write!(f, "string"),
            Type::Record(s) => write!(f, "record {}", s),
            Type::Int => write!(f, "int"),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Default)]
pub enum Value {
    #[default]
    Unit,
    String(String),
    Record(Record),
    Int(i64),
}

impl Value {
    pub fn type_of(&self, resolved: &Resolved) -> Type {
        match self {
            Self::Unit => Type::Unit,
            Self::String(_) => Type::String,
            Self::Record(record) => {
                Type::Record(resolved[resolved[record.schema].name].to_owned())
            }
            Self::Int(_) => Type::Int,
        }
    }

    pub fn expect_string(
        &self,
        resolved: &Resolved,
    ) -> Result<&str, eval::Error> {
        match self {
            Self::String(s) => Ok(&s),
            _ => Err(eval::Error::TypeError {
                expected: Type::String,
                found: self.type_of(resolved),
            }),
        }
    }
}

impl DisplayWith for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>, ctx: DisplayCtx) -> fmt::Result {
        match self {
            Value::Unit => write!(f, "()"),
            Value::String(s) => write!(f, "{}", s),
            Value::Record(record) => write!(f, "{}", record.with_ctx(ctx)),
            Value::Int(i) => write!(f, "{}", i),
        }
    }
}
