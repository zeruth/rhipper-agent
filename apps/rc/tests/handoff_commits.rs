//! A rerun's handoff of a failed test lists the commits since the card's last run that touch the
//! test's file and the card's files, as hash and subject, at most ten.

use std::path::Path;
use std::process::Command;

use ra_context::Handoff;

/// When the last run ended; the commits before it are not new to the rerun.
const RUN_AT: u64 = 1_700_000_000;

fn git(dir: &Path, date: u64, args: &[&str]) {
    let stamp = format!("{date} +0000");
    let out = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .env("GIT_COMMITTER_DATE", &stamp)
        .env("GIT_AUTHOR_DATE", &stamp)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn commit(dir: &Path, date: u64, file: &str, subject: &str) {
    let path = dir.join(file);
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("the dir");
    std::fs::write(&path, subject).expect("a file");
    git(dir, date, &["add", "."]);
    git(dir, date, &["commit", "-q", "-m", subject]);
}

fn failed_run() -> Handoff {
    Handoff {
        when: RUN_AT,
        model: "m".to_owned(),
        outcome: "failed".to_owned(),
        check: String::new(),
        verdict: None,
        changed: Vec::new(),
        report: String::new(),
        warnings: Vec::new(),
        read: Vec::new(),
        failures: vec!["a_thing_works\nthread 'a_thing_works' panicked at tests/thing.rs:12:5:\nboom".to_owned()],
    }
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a tempdir");
    git(dir.path(), RUN_AT - 100, &["init", "-q"]);
    commit(dir.path(), RUN_AT - 100, "tests/thing.rs", "before the run");
    dir
}

#[test]
fn the_commits_since_the_run_that_touch_the_test_or_the_card_are_listed() {
    let dir = project();
    commit(dir.path(), RUN_AT + 10, "tests/thing.rs", "fix the test");
    commit(dir.path(), RUN_AT + 20, "src/card.rs", "change the card file");
    commit(dir.path(), RUN_AT + 30, "src/other.rs", "touch nothing of ours");
    let lines = ra_console::agent::commits_touching_failures(dir.path(), &failed_run(), &["src/card.rs".to_owned()]);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].contains("change the card file"), "{lines:?}");
    assert!(lines[1].contains("fix the test"), "{lines:?}");
    assert!(!lines.iter().any(|l| l.contains("before the run")), "{lines:?}");
}

#[test]
fn at_most_ten_commits_are_listed() {
    let dir = project();
    for n in 0..15 {
        commit(dir.path(), RUN_AT + 10 + n, "tests/thing.rs", &format!("change {n}"));
    }
    let lines = ra_console::agent::commits_touching_failures(dir.path(), &failed_run(), &[]);
    assert_eq!(lines.len(), 10, "{lines:?}");
}

#[test]
fn a_run_that_failed_no_test_lists_no_commits() {
    let dir = project();
    commit(dir.path(), RUN_AT + 10, "src/card.rs", "change the card file");
    let mut run = failed_run();
    run.failures.clear();
    assert!(ra_console::agent::commits_touching_failures(dir.path(), &run, &["src/card.rs".to_owned()]).is_empty());
}
