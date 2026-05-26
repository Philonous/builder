// params positional, then key-value

pub mod base;
pub mod command;
pub mod eval;
pub mod parameters;

use parameters::Parameters;

struct TargetType<'a> {
    parameters: Parameters<'a>,
}

// struct Target<'a> {}

// struct Dict<'a>(HashMap<

// pub struct Target<'a> {
//     pub typ: &'a TargetType,
//     // pub params
// }
