//! Common definitions

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Ident<'a>(pub &'a str);
