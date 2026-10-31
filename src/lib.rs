use std::{
    fs,
    io::{ErrorKind, IsTerminal},
    path::Path,
    sync::Arc,
};

use ariadne::Source;

// mod error;
pub mod analyser;
pub mod common;
pub mod error;
pub mod log;
pub mod model;
pub mod parser;
pub mod process;
pub mod store;

use clap::{Parser, Subcommand};
use model::rule::database::Database;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

use crate::{
    error::Errors,
    model::{
        eval::{self, Resolved, ResolvedRef},
        rule::{ModuleId, Modules},
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
    /// List the targets that can be run
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
    module: ModuleId,
) -> Resolved<lasso::Rodeo> {
    match ast.resolve(module) {
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

pub fn run(
    cwd: &Path,
    src: &str,
    request: impl AsRef<str>,
) -> Result<Vec<model::rule::TargetReport>, error::Spanned<eval::Error>> {
    let request = request.as_ref();
    let ast = parse(&src);
    let mut resolved = resolve(&src, ast, Default::default());
    let database = Database::load(cwd.join("build.db"), true, &mut resolved)
        .unwrap()
        .unwrap();
    let modules = Modules::new();
    let resolved = Resolved {
        functions: resolved.functions,
        resolver: resolved.resolver.into_resolver(),
        records: resolved.records,
        target_types: resolved.target_types,
        rules: resolved.rules,
        modules,
    };

    let rule = match resolved.rules.find_by_target(&request) {
        Ok(t) => t,
        Err(e) => {
            blog!(error, "{}", e);
            std::process::exit(1)
        }
    };
    let target_type = &resolved[rule.kind];
    let resolved_ref = ResolvedRef::from(&resolved);
    let target = target_type
        .target_from_name(resolved_ref, &request)
        .expect("construct target from name");

    let mut scheduler = model::rule::Scheduler::new(cwd.to_owned(), database);

    scheduler.demand_all(resolved_ref, &vec![target])
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
            let _ = resolve(&src, ast, Default::default());
            println!("Source file {:?} checked without errors", &file);
        }
        Command::Run { request } => {
            let cwd = match cli.working_dir {
                None => std::env::current_dir(),
                Some(working_dir) => std::path::absolute(working_dir),
            }
            .expect("cwd");
            let src = Arc::from(read_file(file).as_str());
            let source = Source::from(Arc::clone(&src));

            match run(&cwd, &src, &request) {
                Err(err) => {
                    blog!(error, "Build failed");
                    error::report(&err)
                        .print(&source)
                        .expect("ariadne: print error");
                    std::process::exit(1)
                }
                Ok(res) => {
                    blog!(info, "Build finished");
                    eprintln!("\nTarget report (in starting order):");
                    for report in res {
                        report.pprint(&source);
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
            let resolved = resolve(&src, ast, Default::default());
            for name in resolved.rules.targets() {
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
            let decls = resolve(&src, ast, Default::default());
            println!("{:?}", decls)
        }
    }
}
