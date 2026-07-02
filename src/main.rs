use std::{
    fs,
    io::{ErrorKind, IsTerminal},
    sync::Arc,
};

use ariadne::Source;

// mod error;
mod analyser;
mod common;
mod error;
mod log;
mod model;
mod parser;
mod process;
mod store;
mod test;

use clap::{Parser, Subcommand};
use model::target::database::Database;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

use crate::model::{eval::Resolved, value::WithDisplayCtxExt};
use log::blog;

#[derive(clap::Parser, Debug)]
#[command(name = "myapp")]
struct Cli {
    /// Optional input file; valid before or after the command.
    #[arg(short, long, global = true, value_name = "PATH")]
    file: Option<PathBuf>,

    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[arg(short, long, global = true)]
    quiet: bool,

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

impl Cli {
    fn log_level(&self) -> tracing::Level {
        if self.quiet {
            tracing::Level::ERROR
        } else {
            match self.verbose {
                0 => tracing::Level::WARN,
                1 => tracing::Level::INFO,
                2 => tracing::Level::DEBUG,
                _ => tracing::Level::TRACE,
            }
        }
    }
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

fn resolve<'a>(
    src: &'a str,
    ast: parser::parser::Ast<'a>,
) -> Resolved<lasso::Rodeo> {
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

fn read_file(path: &PathBuf) -> String {
    match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            blog!(error,
                "Source file {} not found, consider passing --file or check that the file exists",
                path.to_string_lossy()
            );
            std::process::exit(1);
        }
        Err(e) => {
            blog!(
                error,
                "Error reading build file {}: {e}",
                path.to_string_lossy()
            );
            std::process::exit(1);
        }
    }
}

fn main() {
    if std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none()
    {
        yansi::enable();
    } else {
        yansi::disable();
    }

    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(cli.log_level().into())
                .from_env_lossy(),
        ) // RUST_LOG=builder=debug
        .with_target(false)
        .init();

    let file = &cli.file.unwrap_or(PathBuf::from("Buildfile"));
    let src = Arc::new(read_file(&file));

    match cli.command {
        Command::Check => {
            let ast = parse(&src);
            let _ = resolve(&src, ast);
            println!("Source file {:?} checked without errors", &file);
        }
        Command::Run { item } => {
            let ast = parse(&src);
            let mut resolved = resolve(&src, ast);
            let (mut database, needs_compaction) = Database::load(
                "build.db",
                true,
                &mut resolved.resolver,
                &resolved.records,
            )
            .unwrap()
            .unwrap();
            let resolved = Resolved {
                resolver: resolved.resolver.into_resolver(),
                functions: resolved.functions,
                records: resolved.records,
                target_types: resolved.target_types,
                targets: resolved.targets,
            };

            if needs_compaction {
                tracing::debug!("Compacting database");
                // TODO: proper error handling
                database.compact(&resolved).expect("compact");
            }

            let target = match resolved.targets.find_by_string(&item) {
                Ok(t) => t,
                Err(e) => {
                    blog!(error, "{}", e);
                    std::process::exit(1)
                }
            };
            let target_type = &resolved[target.kind];
            let request = target_type
                .request_from_name(&resolved, &item)
                .expect("construct request from name");

            let mut scheduler = model::target::Scheduler::new(database);

            match target.run(&resolved, &mut scheduler, &request) {
                Err(e) => {
                    blog!(error, "Build failed:: {}", e);
                    std::process::exit(1)
                }
                Ok(res) => match res.value {
                    model::eval::Value::Unit => (),
                    _ => {
                        blog!(
                            info,
                            "Build finished: {}",
                            res.value.with_ctx(&resolved)
                        )
                    }
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
