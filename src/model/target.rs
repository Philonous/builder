use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::LazyLock;

use super::base::Value;
use super::eval;
use super::parameters;
use super::parameters::{CowSlice, Parameter, Parameters};
use crate::common::Ident;

#[derive(Debug, Default)]
pub struct TargetType<'a> {
    pub name: Ident<'a>,
    pub name_is: Ident<'a>,
    pub parameters: parameters::Parameters<'a>,
    // Todo: Here go verifying trace info etc.
}

// This will have to be dynamically added if we want to intern idents
pub const TARGET_TYPE_FILE: TargetType<'static> = TargetType {
    name: Ident("file"),
    name_is: Ident("path"),
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
    name_is: Ident("name"),
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
    pub name: String,
    pub args: HashMap<Ident<'a>, Value<'a>>,

    pub commands: eval::Block<'a>,
}

impl<'a> Default for Target<'a> {
    fn default() -> Self {
        Self {
            kind: &TARGET_TYPE_TASK,
            name: Default::default(),
            args: Default::default(),
            commands: Default::default(),
        }
    }
}

impl<'a> Target<'a> {}
