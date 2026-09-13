// Rust test module that has a number of tests, each parsing

// First test should create a temporary directory, copy all files from the source directory (hard-coded for the test),
// run crate::main::run(source_dir), then leave space for the actual test

use std::fs;
use std::path::Path;
use tempfile::TempDir;

mod project;

use project::{Project, Status};

#[test]

// Cold build runs everything
fn simple_run() {
    // Create temporary directory
    let p = Project::new(
        r#"
target file("1") {
  write_file("1", "write_file");
}
    "#,
    );

    let r = p.run("1");
    r.assert_ok();

    assert_eq!(r.status("./1"), Status::Ok);
    assert_eq!(p.read("1"), "write_file");
}

#[test]
fn skip_on_unchanged() {
    // Create temporary directory
    let p = Project::new(
        r#"
target file("1") {
  write_file("1", "test");
}
    "#,
    );

    let r1 = p.run("1");
    r1.assert_ok();

    let r2 = p.run("1");
    r2.assert_ok();
    assert_eq!(r2.status("./1"), Status::Skipped);
}

#[test]
fn rerun_on_change() {
    // Create temporary directory
    let p = Project::new(
        r#"
target file("1") {
  write_file("1", "test");
  write_file("sentinel", append = true, val = "." );
}
    "#,
    );

    let r1 = p.run("1");
    r1.assert_ok();

    // Modify file to trigger rebuild
    p.write("1", "toodeloo");

    // Target should run again
    let r2 = p.run("1");
    r2.assert_ok();

    assert_eq!(r2.status("./1"), Status::Ok);
    // Check that the file has been written to twice
    // - checks that the target has actually been run
    assert_eq!(p.read("sentinel"), "..");
}

#[test]
fn rerun_on_delete() {
    // Create temporary directory
    let p = Project::new(
        r#"
target file("1") {
  write_file("1", "test");
  write_file("sentinel", append = true, val = "." );
}
    "#,
    );

    let r1 = p.run("1");
    r1.assert_ok();

    // Modify file to trigger rebuild
    p.rm("1");

    // Target should run again
    let r2 = p.run("1");
    r2.assert_ok();

    assert_eq!(r2.status("./1"), Status::Ok);
    // Check that the file has been written to twice
    // - checks that the target has actually been run
    assert_eq!(p.read("sentinel"), "..");
}

// #[test]
// fn rerun_on_dep_changed() {
//     // Create temporary directory
//     let p = Project::new(
//         r#"
// target file("1") {
//   need(file("dep"));
//   write_file("1", "test");
// }
//     "#,
//     );

//     p.write("dep", "v1");

//     let r1 = p.run("1");
//     r1.assert_ok();

//     // Modify file to trigger rebuild
//     p.write("dep", "toodeloo");

//     // Target should run again
//     let r2 = p.run("1");
//     r2.assert_ok();

//     assert_eq!(r2.status("./1"), Status::Ok);
// }

// #[test]
// fn transitive_dependency() {
//     // Create temporary directory
//     let p = Project::new(
//         r#"
// target file("1") {
//   need(file("dep"));
//   `cp dep 1`
// }

// target file("2") {
//   need(file("1"));
//   `cp 1 2`
// }
//     "#,
//     );

//     p.write("dep", "v1");

//     let r1 = p.run("1");
//     r1.assert_ok();

//     // Modify file to trigger rebuild
//     p.write("dep", "toodeloo");

//     // Target should run again
//     let r2 = p.run("1");
//     r2.assert_ok();

//     assert_eq!(r2.status("./2"), Status::Ok);
// }
