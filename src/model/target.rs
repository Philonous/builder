use std::borrow::Cow;

use super::model::eval;

use crate::parser::parser;
use std::process;

pub struct Target<'a> {
    pub typ: Ident<'a>,
    pub name: Cow<'a, str>,
    commands: eval::Block<'a>,
}

impl<'a> Target<'a> {}
