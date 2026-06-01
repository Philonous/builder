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

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(clap::Parser, Debug)]
#[command(
    name = "myapp",
    args_conflicts_with_subcommands = true,  // top-level args XOR a subcommand
    subcommand_negates_reqs = true           // a subcommand relaxes `run_args` requirement
)]
struct Cli {
    /// Optional input file; valid before or after the command.
    #[arg(short, long, global = true, value_name = "PATH")]
    file: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,

    /// Positionals for the implicit `run` (when no command is given).
    #[arg(required = true)]
    run_args: Vec<String>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Validate; takes no arguments.
    Check,

    /// Dump zero or more items.
    Dump,

    /// Run one or more items (the default command).
    Run {
        #[arg(required = true)]
        items: Vec<String>,
    },
    /// Run one or more items (the default command).
    Targets,
}

fn main() {
    let Cli {
        file,
        command,
        run_args,
    } = Cli::parse();
    let file = &file.unwrap_or(PathBuf::from("Buildfile"));
    let command = command.unwrap_or(Command::Run { items: run_args });
    let src = fs::read_to_string(&file).expect("Failed to read file");
    let ast = match parser::parser::parse_str(&src) {
        Ok(ast) => ast,
        Err(errs) => {
            for err in errs {
                error::report(&err).print(Source::from(&src));
            }
            std::process::exit(1);
        }
    };

    let decls = match ast.resolve() {
        Ok(decls) => decls,
        Err(error::Errors(errs)) => {
            for err in errs {
                error::report(&err).print(Source::from(&src));
            }
            std::process::exit(1);
        }
    };

    match command {
        Command::Check => println!("Source file={:?} checked", &file),
        Command::Dump => {
            println!("{:?}", decls)
        }
        Command::Run { items } => for item in items {},
        Command::Targets => {
            for decl in decls {
                println!("{:?}", decl.name);
            }
        }
    }
}
