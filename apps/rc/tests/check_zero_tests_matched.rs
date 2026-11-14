//! A check whose `--test` target ran 0 tests reports a missing target, not a failure.

use std::path::Path;
use std::process::{Command, Output};

fn rc(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rc")).args(args).env_remove("RHIPPER_SCRATCH").current_dir(dir).output().expect("rc runs")
}

/// A project whose `gamma` test target holds no test, and a task checked by it; the task's id.
fn project_with(dir: &Path, acceptance: &str) -> String {
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n")
        .expect("a manifest");
    std::fs::create_dir_all(dir.join("src")).expect("src");
    std::fs::create_dir_all(dir.join("tests")).expect("tests");
    std::fs::write(dir.join("src").join("lib.rs"), "").expect("a lib");
    std::fs::write(dir.join("tests").join("gamma.rs"), "").expect("an empty target");
    let out = rc(dir, &["init", "--empty"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let id_of = |args: &[&str]| {
        let out = rc(dir, args);
        assert!(out.status.success(), "rc {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.lines().next().and_then(|l| l.strip_prefix('#')).and_then(|l| l.split_whitespace().next()).expect("an id").to_owned()
    };
    let project = id_of(&["add", "project", "Alpha"]);
    let milestone = id_of(&["add", "milestone", "M", "--parent", &project]);
    id_of(&["add", "task", "T", "--parent", &milestone, "--acceptance", acceptance])
}

#[test]
fn a_target_that_ran_no_test_waits_on_main_and_is_not_failed() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let card = project_with(dir.path(), "`cargo test --test gamma`");
    let out = rc(dir.path(), &["check", &card]);
    let shown = String::from_utf8_lossy(&out.stdout);
    assert!(shown.starts_with("waits on main: test target gamma not in the worktree"), "{shown}");
    assert!(!shown.starts_with("failed"), "{shown}");
}
