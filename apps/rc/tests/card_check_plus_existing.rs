//! `rc check` flags a `+` file of a card that already exists in the tree and drops the mark
//! from the card, and leaves a `+` file that is missing alone.

use std::path::Path;
use std::process::Command;

fn rc(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rc")).args(args).current_dir(dir).output().expect("rc runs")
}

fn project(files: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a tempdir");
    assert!(rc(dir.path(), &["init", "--empty"]).status.success());
    assert!(rc(dir.path(), &["add", "project", "Alpha"]).status.success());
    let mut args = vec!["add", "task", "Change", "--parent", "1", "--objective", "Change it", "--files"];
    args.extend(files.split_whitespace());
    let added = rc(dir.path(), &args);
    assert!(added.status.success(), "{}", String::from_utf8_lossy(&added.stderr));
    dir
}

fn check_says(dir: &Path) -> String {
    String::from_utf8_lossy(&rc(dir, &["check"]).stdout).into_owned()
}

#[test]
fn a_plus_file_that_exists_is_flagged_without_its_plus() {
    let dir = project("+src/a.rs");
    std::fs::create_dir_all(dir.path().join("src")).expect("the dir");
    std::fs::write(dir.path().join("src/a.rs"), "fn a() {}\n").expect("a source");
    let said = check_says(dir.path());
    assert!(said.contains("marks src/a.rs as new, but it is in the tree"), "{said}");
}

#[test]
fn a_plus_file_that_exists_loses_its_plus_on_the_card() {
    let dir = project("+src/a.rs +src/b.rs");
    std::fs::create_dir_all(dir.path().join("src")).expect("the dir");
    std::fs::write(dir.path().join("src/a.rs"), "fn a() {}\n").expect("a source");
    check_says(dir.path());
    let roadmap = std::fs::read_to_string(dir.path().join("rhipper.md")).expect("the roadmap");
    assert!(roadmap.contains("files: src/a.rs +src/b.rs"), "{roadmap}");
    assert!(!check_says(dir.path()).contains("but it is in the tree"));
}

#[test]
fn a_plus_file_that_is_missing_keeps_its_plus() {
    let dir = project("+src/a.rs");
    check_says(dir.path());
    let roadmap = std::fs::read_to_string(dir.path().join("rhipper.md")).expect("the roadmap");
    assert!(roadmap.contains("files: +src/a.rs"), "{roadmap}");
}

#[test]
fn a_plus_file_that_is_missing_is_not_flagged() {
    let dir = project("+src/a.rs");
    let said = check_says(dir.path());
    assert!(!said.contains("but it is in the tree"), "{said}");
}
