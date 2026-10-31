use std::sync::LazyLock;

use builder;
use builder::model::rule::{ReportResult, TargetReport};
use tracing_subscriber::EnvFilter;

static TRACING: LazyLock<()> = LazyLock::new(|| {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::builder().from_env_lossy()) // RUST_LOG=builder=debug
        .with_target(false)
        .with_test_writer()
        .init();
});

fn init_tracing() {
    LazyLock::force(&TRACING);
}

#[derive(Debug, PartialEq)]
pub enum Status {
    Ok,
    OkClean,
    Skipped,
    Cancelled,
    Error,
}

pub struct Report(Vec<TargetReport>);

impl Report {
    pub fn find(&self, name: &str) -> &TargetReport {
        match self.0.iter().find(|tr| &tr.target == name) {
            Some(report) => report,
            None => {
                eprintln!("No target report for {name} found");
                eprintln!("Available reports:");
                for r in self.0.iter() {
                    eprintln!("  {}", r.target);
                }
                panic!("")
            }
        }
    }
    pub fn status(&self, name: &str) -> Status {
        match &self.find(name).result {
            ReportResult::Success { clean, value } => {
                if *clean {
                    Status::OkClean
                } else {
                    Status::Ok
                }
            }
            ReportResult::Skipped => Status::Skipped,
            ReportResult::Cancelled => Status::Cancelled,
            ReportResult::Error { error } => Status::Error,
        }
    }
    pub fn assert_ok(&self) {
        for tr in self.0.iter() {
            if matches!(
                tr.result,
                ReportResult::Cancelled | ReportResult::Error { .. }
            ) {
                panic!("Build failed but was expected to succeed")
            }
        }
    }
}

pub struct Project {
    dir: tempfile::TempDir,
}

impl Project {
    pub fn new(buildfile: impl AsRef<str>) -> Self {
        init_tracing();
        let dir = tempfile::tempdir().expect("temporary directory");
        let this = Self { dir };
        this.write("Buildfile", buildfile.as_ref());
        this
    }

    pub fn write(&self, path: &str, contents: &str) {
        std::fs::write(self.dir.as_ref().join(path), contents)
            .expect("write buildfile");
    }

    pub fn read(&self, path: &str) -> String {
        std::fs::read_to_string(self.dir.as_ref().join(path))
            .expect("read file")
    }

    pub fn rm(&self, path: &str) {
        std::fs::remove_file(self.dir.as_ref().join(path)).unwrap();
    }

    pub fn run(&self, request: impl AsRef<str>) -> Report {
        let src = self.read("Buildfile");
        let res = builder::run(self.dir.as_ref(), &src, request).unwrap();
        Report(res)
    }

    // fn touch(&self, path: &str);
    // fn remove(&self, path: &str);
    // fn exists(&self, path: &str) -> bool;
}
