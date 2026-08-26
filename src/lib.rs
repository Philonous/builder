use std::{
    fs,
    io::{ErrorKind, IsTerminal},
    path::Path,
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

use crate::{
    error::Errors,
    model::{
        eval::{self, Resolved},
        value::WithDisplayCtxExt,
    },
};
use log::blog;

#[derive(clap::Parser, Debug)]
#[command(name = "myapp")]
struct Cli {
    /// Optional input file; valid before or after the command.
    #[arg(short, long, global = true, value_name = "PATH")]
    file: Option<PathBuf>,

    #[arg(short, long, global = true, value_name = "PATH")]
    working_dir: Option<PathBuf>,

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
        request: String,
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
    workdir: &Path,
) -> Resolved<lasso::Rodeo> {
    match ast.resolve(workdir) {
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

fn read_file(path: &Path) -> String {
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

pub fn main() {
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

    match cli.command {
        Command::Check => {
            let cwd = match cli.working_dir {
                None => std::env::current_dir(),
                Some(working_dir) => std::path::absolute(working_dir),
            }
            .expect("cwd");
            let src = Arc::new(read_file(&file));
            let ast = parse(&src);
            let _ = resolve(&src, ast, &cwd);
            println!("Source file {:?} checked without errors", &file);
        }
        Command::Run { request } => {
            let src = Arc::new(read_file(file));
            let source = Source::from(src.as_ref());
            let cwd = match cli.working_dir {
                None => std::env::current_dir(),
                Some(working_dir) => std::path::absolute(working_dir),
            }
            .expect("cwd");

            let ast = parse(&src);
            let mut resolved = resolve(&src, ast, &cwd);
            let database = Database::load(
                cwd.join("build.db"),
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

            let target = match resolved.targets.find_by_string(&request) {
                Ok(t) => t,
                Err(e) => {
                    blog!(error, "{}", e);
                    std::process::exit(1)
                }
            };
            let target_type = &resolved[target.kind];
            let request = target_type
                .request_from_name(&resolved, &request)
                .expect("construct request from name");

            let mut scheduler = model::target::Scheduler::new(database);

            match scheduler.demand_all(&resolved, &vec![request]) {
                Err(err) => {
                    blog!(error, "Build failed");
                    error::report(&err)
                        .print(&source)
                        .expect("ariadne: print error");
                    std::process::exit(1)
                }
                Ok(res) => {
                    blog!(info, "Build finished");
                    for report in res {
                        eprint!("{}: ", report.request.with_ctx(&resolved));
                        match report.result {
                            model::target::ReportResult::Success {
                                clean,
                                value,
                            } => eprintln!("completed successfully"),
                            model::target::ReportResult::Skipped => {
                                eprintln!("did not need to be run")
                            }
                            model::target::ReportResult::Cancelled => {
                                eprintln!("was cancelled")
                            }
                            model::target::ReportResult::Error { error } => {
                                eprintln!("");
                                error::report(&error)
                                    .print(&source)
                                    .expect("ariadne: print error");
                            }
                        }
                    }
                }
            };
        }
        Command::Targets => {
            let cwd = match cli.working_dir {
                None => std::env::current_dir(),
                Some(working_dir) => std::path::absolute(working_dir),
            }
            .expect("cwd");
            let src = Arc::new(read_file(&file));
            let ast = parse(&src);
            let resolved = resolve(&src, ast, &cwd);
            for name in resolved.targets.names() {
                println!("{:?}", name);
            }
        }

        Command::DumpTokens => {
            let src = Arc::new(read_file(&file));
            let tokens: Vec<(Result<parser::lexer::Token, _>, logos::Span)> =
                parser::lexer::Tokenizer::new(src.as_str()).collect();
            for tok in tokens {
                println!("{:?}", tok);
            }
        }

        Command::DumpParse => {
            let src = Arc::new(read_file(&file));
            let ast = parse(&src);
            println!("{:?}", ast)
        }

        Command::Dump => {
            let src = Arc::new(read_file(&file));
            let cwd = match cli.working_dir {
                None => std::env::current_dir(),
                Some(working_dir) => std::path::absolute(working_dir),
            }
            .expect("cwd");
            let ast = parse(&src);
            let decls = resolve(&src, ast, &cwd);
            println!("{:?}", decls)
        }
    }
}
