use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::LazyLock;

use super::base::Value;
use super::eval;
use super::parameters;
use super::parameters::{CowSlice, Parameter, Parameters, Patterns};
use crate::common::Ident;
use crate::error;

#[derive(Debug, Default)]
pub struct TargetType<'a> {
    pub name: Ident<'a>,
    // Parameter number
    pub name_is: usize,
    pub parameters: Parameters<'a>,
    // Todo: Here go verifying trace info etc.
}

// This will have to be dynamically added if we want to intern idents
pub const TARGET_TYPE_FILE: TargetType<'static> = TargetType {
    name: Ident("file"),
    name_is: 0, // path
    parameters: Parameters(CowSlice::Borrowed(&[(
        Ident("path"),
        Parameter {
            default: None,
            span: None,
        },
    )])),
};
// This will have to be dynamically added if we want to intern idents
pub const TARGET_TYPE_TASK: TargetType<'static> = TargetType {
    name: Ident("task"),
    name_is: 0,
    parameters: Parameters(CowSlice::Borrowed(&[(
        Ident("name"),
        Parameter {
            default: None,
            span: None,
        },
    )])),
};

#[derive(Debug)]
pub struct Target<'a> {
    pub kind: &'a TargetType<'a>,
    pub name: Option<String>,
    pub parameters: Patterns<'a>,
    // Resolved variable slots for running the block
    pub slots: eval::Slots<'a>,
    pub commands: eval::Block<'a>,
}

// Dummy empty target so we can continue analysis
impl<'a> Default for Target<'a> {
    fn default() -> Self {
        Self {
            kind: &TARGET_TYPE_TASK,
            name: Default::default(),
            parameters: Default::default(),
            slots: Default::default(),
            commands: Default::default(),
        }
    }
}

impl<'a> Target<'a> {
    fn run(&self) -> Result<Value<'a>, error::Spanned<eval::Error>> {
        let mut vars = Vec::new();
        let mut ctx = eval::EvalContext {
            variables: &mut vars,
        };
        self.commands.run(&mut ctx)
    }
}
