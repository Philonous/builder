pub use crate::common::Ident;
use std::borrow::Cow;

impl<'a> Ident<'a> {
    fn to_string(&self) -> String {
        self.0.to_owned()
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub enum Value<'a> {
    Unit,
    String(Cow<'a, str>),
}

impl<'a> std::fmt::Display for Value<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Unit => write!(f, "()"),
            // Todo: String quoting
            Value::String(cow) => write!(f, "{}", *cow),
        }
    }
}
