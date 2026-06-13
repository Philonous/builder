use std::{fs, sync::Arc};

use ariadne::Source;

// mod error;
mod analyser;
mod common;
mod error;
mod model;
mod parser;
mod process;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::model::{eval::Resolved, value::WithDisplayCtxExt};

#[derive(clap::Parser, Debug)]
#[command(name = "myapp")]
struct Cli {
    /// Optional input file; valid before or after the command.
    #[arg(short, long, global = true, value_name = "PATH")]
    file: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Validate; takes no arguments.
    Check,

    /// Dump tokens.
    DumpTokens,

    /// Dump zero or more items.
    DumpParse,

    Dump,

    /// Run one or more items
    Run {
        #[arg(required = true)]
        item: String,
    },
    /// Run one or more items (the default command).
    Targets,
}

fn parse(src: &str) -> parser::parser::Ast<'_> {
    match parser::parser::parse_str(&src) {
        Ok(ast) => ast,
        Err(errs) => {
            for err in errs {
                error::report(&err)
                    .print(Source::from(&src))
                    .expect("ariadne: print error");
            }
            std::process::exit(1);
        }
    }
}

fn resolve<'a>(src: &'a str, ast: parser::parser::Ast<'a>) -> Resolved {
    match ast.resolve() {
        Ok(decls) => decls,
        Err(error::Errors(errs)) => {
            for err in errs {
                error::report(&err)
                    .print(Source::from(&src))
                    .expect("ariadne: print error");
            }
            std::process::exit(1);
        }
    }
}

fn main() {
    let Cli { file, command } = Cli::parse();
    let file = &file.unwrap_or(PathBuf::from("Buildfile"));
    let src = Arc::new(fs::read_to_string(&file).expect("Failed to read file"));

    match command {
        Command::Check => {
            let _ = parse(&src);
            println!("Source file={:?} checked", &file);
        }
        Command::Run { item } => {
            let ast = parse(&src);
            let resolved = resolve(&src, ast);
            let target = match resolved.targets.find_by_string(&item) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("{}", e);
                    std::process::exit(1)
                }
            };
            match target.run(&resolved) {
                Err(e) => {
                    eprintln!("{}", e);
                    std::process::exit(1)
                }
                Ok(val) => match val {
                    model::eval::Value::Unit => (),
                    _ => println!("{}", val.with_ctx(&resolved)),
                },
            }
        }
        Command::Targets => {
            let ast = parse(&src);
            let resolved = resolve(&src, ast);
            for name in resolved.targets.names() {
                println!("{:?}", name);
            }
        }

        Command::DumpTokens => {
            let tokens: Vec<(Result<parser::lexer::Token, _>, logos::Span)> =
                parser::lexer::Tokenizer::new(src.as_str()).collect();
            for tok in tokens {
                println!("{:?}", tok);
            }
        }

        Command::DumpParse => {
            let ast = parse(&src);
            println!("{:?}", ast)
        }

        Command::Dump => {
            let ast = parse(&src);
            let decls = resolve(&src, ast);
            println!("{:?}", decls)
        }
    }
}
