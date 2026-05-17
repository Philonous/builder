use std::env;
use std::fs;

use ariadne::{Color, Label, Report, ReportKind, Source};

mod parser;

fn main() {
    let filename = env::args().nth(1).expect("Expected file argument");
    let src = fs::read_to_string(&filename).expect("Failed to read file");

    match parser::parser::parse_str(&src) {
        Ok(expr) => {
            println!("{:?}", expr)
        }
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
        }
    }
}
