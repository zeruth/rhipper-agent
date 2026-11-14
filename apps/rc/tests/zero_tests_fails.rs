//! A check whose test filter ran 0 tests is failed, with the line "the filter matched no test
//! name", not passed.

use std::path::Path;
use std::process::{Command, Output};

fn rc(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rc")).args(args).env_remove("RHIPPER_SCRATCH").current_dir(dir).output().expect("rc runs")
}

/// A project holding a one-test crate and a task whose acceptance is `acceptance`; the task's id.
fn project_with(dir: &Path, acceptance: &str) -> String {
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n")
        .expect("a manifest");
    std::fs::create_dir_all(dir.join("src")).expect("src");
    std::fs::write(dir.join("src").join("lib.rs"), "#[test]\nfn it_runs() {}\n").expect("a lib");
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

fn kept_log(out: &Output) -> String {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let path = stdout.lines().last().expect("a line");
    std::fs::read_to_string(path).expect("the log")
}

#[test]
fn a_filter_that_matches_no_test_fails_the_check() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let card = project_with(dir.path(), "`cargo test no_such_test_name_xyz`");
    let out = rc(dir.path(), &["check", &card]);
    assert_eq!(out.status.code(), Some(1), "{}", String::from_utf8_lossy(&out.stdout));
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("failed"), "{}", String::from_utf8_lossy(&out.stdout));
    let kept = kept_log(&out);
    assert!(kept.contains("running 0 tests"), "{kept}");
    assert!(kept.trim_end().ends_with("the filter matched no test name"), "{kept}");
}

#[test]
fn a_filter_that_matches_a_test_passes_the_check() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let card = project_with(dir.path(), "`cargo test it_runs`");
    let out = rc(dir.path(), &["check", &card]);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stdout));
    let kept = kept_log(&out);
    assert!(kept.contains("running 1 test"), "{kept}");
    assert!(!kept.contains("the filter matched no test name"), "{kept}");
}
