//! `rc check` flags an objective that adds a backticked `name:` line or field while neither the
//! objective nor the notes says which section or struct holds it, and names the field.

use std::path::Path;
use std::process::Command;

fn rc(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rc")).args(args).current_dir(dir).output().expect("rc runs")
}

fn project_with(objective: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a tempdir");
    assert!(rc(dir.path(), &["init", "--empty"]).status.success());
    assert!(rc(dir.path(), &["add", "project", "Alpha"]).status.success());
    let added = rc(dir.path(), &["add", "task", "Fix", "--parent", "1", "--objective", objective]);
    assert!(added.status.success(), "{}", String::from_utf8_lossy(&added.stderr));
    dir
}

fn check_of(dir: &Path) -> String {
    String::from_utf8_lossy(&rc(dir, &["check"]).stdout).into_owned()
}

#[test]
fn an_added_line_with_no_home_is_flagged_by_name() {
    let dir = project_with("Add an `owner:` line to every card.");
    let shown = check_of(dir.path());
    assert!(shown.contains("#3: the objective adds `owner:` without saying which section or struct holds it"), "{shown}");
}

#[test]
fn a_line_placed_in_a_section_or_struct_is_clean() {
    let dir = project_with("Add an `owner:` line to the Details section of every card.");
    let shown = check_of(dir.path());
    assert!(!shown.contains("`owner:`"), "{shown}");

    let dir = project_with("Add an `owner:` field to the Card struct.");
    let shown = check_of(dir.path());
    assert!(!shown.contains("`owner:`"), "{shown}");
}

#[test]
fn an_objective_that_only_mentions_a_line_is_clean() {
    let dir = project_with("Read the `owner:` line of each card.");
    let shown = check_of(dir.path());
    assert!(!shown.contains("`owner:`"), "{shown}");
}
