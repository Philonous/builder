//! Common definitions

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Ident<'a>(pub &'a str);

impl<'a> std::fmt::Display for Ident<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl<'a> Default for Ident<'a> {
    fn default() -> Self {
        Self("")
    }
}
