//! `rc check` flags an acceptance that reads generated fixtures no `files:` entry creates, until
//! the card carries a `generator:` line.

use std::path::Path;
use std::process::Command;

fn rc(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rc")).args(args).current_dir(dir).output().expect("rc runs")
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a tempdir");
    assert!(rc(dir.path(), &["init", "--empty"]).status.success());
    assert!(rc(dir.path(), &["add", "project", "Alpha"]).status.success());
    dir
}

fn check_of(dir: &Path) -> String {
    String::from_utf8_lossy(&rc(dir, &["check"]).stdout).into_owned()
}

const ACCEPTANCE: &str = "the decoder matches tests/generated/case1.bin";

#[test]
fn generated_fixtures_without_a_generator_line_are_flagged() {
    let dir = project();
    let added = rc(dir.path(), &["add", "task", "Fix", "--parent", "1", "--acceptance", ACCEPTANCE]);
    assert!(added.status.success(), "{}", String::from_utf8_lossy(&added.stderr));
    let shown = check_of(dir.path());
    assert!(shown.contains("`generator:` line"), "{shown}");
}

#[test]
fn a_generator_line_clears_the_flag() {
    let dir = project();
    let added = rc(
        dir.path(),
        &[
            "add",
            "task",
            "Fix",
            "--parent",
            "1",
            "--objective",
            "Decode.\ngenerator: python tools/gen.py (worker)",
            "--acceptance",
            ACCEPTANCE,
        ],
    );
    assert!(added.status.success(), "{}", String::from_utf8_lossy(&added.stderr));
    let shown = check_of(dir.path());
    assert!(!shown.contains("`generator:` line"), "{shown}");
}
