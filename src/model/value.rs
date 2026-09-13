pub mod display;
pub mod record;
pub mod serialize;

use std::fmt;

use super::eval::{self, Resolved, ResolvedRef};
pub use display::{DisplayWith, WithDisplayCtxExt};
use lasso::Spur;
pub use record::{Record, RecordPattern, RecordSchema, RecordSchemas};

#[derive(Eq, Hash, PartialEq, Debug, Clone, Copy)]
pub struct Ident(pub Spur);

impl From<Spur> for Ident {
    fn from(value: Spur) -> Self {
        Self(value)
    }
}

impl Default for Ident {
    fn default() -> Self {
        Self(Default::default())
    }
}

impl DisplayWith for Ident {
    fn fmt(
        &self,
        f: &mut fmt::Formatter<'_>,
        ctx: ResolvedRef<'_>,
    ) -> fmt::Result {
        write!(f, "{}", &ctx[*self])
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Type {
    Unit,
    String,
    Record,
    Int,
    Bool,
}

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Unit => write!(f, "()"),
            Type::String => write!(f, "string"),
            Type::Record => write!(f, "record"),
            Type::Int => write!(f, "int"),
            Type::Bool => write!(f, "bool"),
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
    Bool(bool),
}

impl Value {
    pub fn type_of(&self) -> Type {
        match self {
            Self::Unit => Type::Unit,
            Self::String(_) => Type::String,
            Self::Record(_record) => Type::Record,
            Self::Int(_) => Type::Int,
            Self::Bool(_) => Type::Bool,
        }
    }

    pub fn expect_string(
        &self,
        resolved: ResolvedRef<'_>,
    ) -> Result<&str, eval::Error> {
        match self {
            Self::String(s) => Ok(&s),
            _ => Err(eval::Error::TypeError {
                expected: Type::String,
                found: self.type_of(),
                display_value: self.with_ctx(resolved).to_string(),
            }),
        }
    }

    pub fn expect_bool(
        &self,
        resolved: ResolvedRef<'_>,
    ) -> Result<bool, eval::Error> {
        match self {
            Self::Bool(s) => Ok(*s),
            _ => Err(eval::Error::TypeError {
                expected: Type::Bool,
                found: self.type_of(),
                display_value: self.with_ctx(resolved).to_string(),
            }),
        }
    }

    pub fn expect_record(
        &self,
        resolved: ResolvedRef<'_>,
    ) -> Result<&Record, eval::Error> {
        match self {
            Self::Record(r) => Ok(&r),
            _ => Err(eval::Error::TypeError {
                expected: Type::Record,
                found: self.type_of(),
                display_value: self.with_ctx(resolved).to_string(),
            }),
        }
    }
}

impl DisplayWith for Value {
    fn fmt(
        &self,
        f: &mut fmt::Formatter<'_>,
        ctx: ResolvedRef<'_>,
    ) -> fmt::Result {
        match self {
            Value::Unit => write!(f, "()"),
            Value::String(s) => write!(f, "{}", s),
            Value::Record(record) => write!(f, "{}", record.with_ctx(ctx)),
            Value::Int(i) => write!(f, "{}", i),
            Value::Bool(b) => {
                write!(f, "{}", if *b { "true" } else { "false" })
            }
        }
    }
}
