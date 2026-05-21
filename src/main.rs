use std::env;
use std::fs;

use ariadne::{Color, Label, Report, ReportKind, Source};

// mod error;
mod parser;
mod target;

fn parse_targets<'a>(src: &'a str) -> Vec<target::Target<'a>> {
    match parser::parser::parse_str(src) {
        Ok(ast) => ast.to_targets(),
        Err(errs) => {
            for err in errs {
                Report::build(ReportKind::Error, ((), err.span().into_range()))
                    .with_config(
                        ariadne::Config::new()
                            .with_index_type(ariadne::IndexType::Byte),
                    )
                    .with_code(3)
                    .with_message(err.to_string())
                    .with_label(
                        Label::new(((), err.span().into_range()))
                            .with_message(err.reason().to_string())
                            .with_color(Color::Red),
                    )
                    .finish()
                    .eprint(Source::from(&src))
                    .unwrap();
            }
            std::process::exit(1);
        }
    }
}

fn main() {
    let args = env::args().collect::<Vec<_>>();
    let filename = &args[1];
    let src = fs::read_to_string(filename).expect("Failed to read file");
    let targets = parse_targets(&src);
    if args.len() >= 3 {
        let target = &args[2];
        for t in targets {
            if &t.name == target {
                println!("found");
                std::process::exit(0);
            }
        }
    } else {
        println!("targets:");
        for target in targets {
            println!(" - {} {}", target.typ.0, target.name);
        }
    }
}
