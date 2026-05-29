//! Common definitions

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Ident<'a>(pub &'a str);

impl<'a> Default for Ident<'a> {
    fn default() -> Self {
        Self("")
    }
}
