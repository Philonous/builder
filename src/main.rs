use std::env;
use std::fs;

use ariadne::{Color, Label, Report, ReportKind, Source};

// mod error;
mod analyser;
mod common;
mod error;
mod model;
mod parser;
mod process;

// fn parse_targets<'a>(src: &'a str) -> Vec<target::Target<'a>> {
//     match parser::parser::parse_str(src) {
//         Ok(ast) => ast.to_targets(),
//         Err(errs) => {
//             for err in errs {
//                 parser::parser::report(&err)
//                     .eprint(Source::from(&src))
//                     .unwrap();
//             }
//             std::process::exit(1);
//         }
//     }
// }

fn main() {
    // let args = env::args().collect::<Vec<_>>();
    // let filename = &args[1];
    // let src = fs::read_to_string(filename).expect("Failed to read file");
    // let targets = parse_targets(&src);
    // if args.len() >= 3 {
    //     let target = &args[2];
    //     for t in targets {
    //         if &t.name == target {
    //             t.exec();
    //             std::process::exit(0);
    //         }
    //     }
    // } else {
    //     println!("targets:");
    //     for target in targets {
    //         println!(" - {} {}", target.typ.0, target.name);
    //     }
    // }
}
