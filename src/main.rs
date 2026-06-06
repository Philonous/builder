use std::fs;

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

    /// Dump zero or more items.
    Dump,

    /// Dump tokens.
    DumpTokens,

    /// Run one or more items
    Run {
        #[arg(required = true)]
        item: String,
    },
    /// Run one or more items (the default command).
    Targets,
}

fn parse(src: &str) -> model::decl::Decls<'_> {
    let ast = match parser::parser::parse_str(&src) {
        Ok(ast) => ast,
        Err(errs) => {
            for err in errs {
                error::report(&err)
                    .print(Source::from(&src))
                    .expect("ariadne: print error");
            }
            std::process::exit(1);
        }
    };

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
    let src = fs::read_to_string(&file).expect("Failed to read file");

    match command {
        Command::Check => {
            let _ = parse(&src);
            println!("Source file={:?} checked", &file);
        }
        Command::Dump => {
            let decls = parse(&src);
            println!("{:?}", decls)
        }
        Command::Run { item } => {
            let decls = parse(&src);
            let target = match decls.find_target_by_string(&item) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("{}", e);
                    std::process::exit(1)
                }
            };
            match target.run() {
                Err(e) => {
                    eprintln!("{}", e);
                    std::process::exit(1)
                }
                Ok(val) => println!("{}", val),
            }
        }
        Command::Targets => {
            let decls = parse(&src);
            for decl in decls.0 {
                println!("{:?}", decl.name);
            }
        }
        Command::DumpTokens => {
            let tokens: Vec<(Result<parser::lexer::Token, _>, logos::Span)> =
                parser::lexer::Tokenizer::new(src.as_str()).collect();
            for tok in tokens {
                println!("{:?}", tok);
            }
        }
    }
}
