//! A check brings main into the card's worktree when it is behind, so a stale branch is not a
//! false red; a merge that conflicts is reported as a conflict and the tree is left as it was.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn commit(dir: &Path, file: &str, text: &str) {
    std::fs::write(dir.join(file), text).unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", file]);
}

fn rc(dir: &Path, target: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rc"))
        .args(args)
        .env_remove("RHIPPER_SCRATCH")
        .env("CARGO_TARGET_DIR", target)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .current_dir(dir)
        .output()
        .expect("rc runs")
}

/// A project on `main` with a card whose check is a test of the probe crate, on branch `card`.
fn project() -> (tempfile::TempDir, PathBuf, PathBuf, String) {
    let dir = tempfile::tempdir().expect("a tempdir");
    let (work, target) = (dir.path().join("work"), dir.path().join("target"));
    std::fs::create_dir_all(work.join("src")).unwrap();
    std::fs::write(work.join("Cargo.toml"), "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n")
        .unwrap();
    std::fs::write(work.join("src").join("lib.rs"), "#[test]\nfn it_runs() {}\n").unwrap();
    std::fs::write(work.join(".gitignore"), "/var/\n/target/\nrhipper.md\n").unwrap();
    git(&work, &["init", "-q", "-b", "main"]);
    let out = rc(&work, &target, &["init", "--empty"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let id_of = |args: &[&str]| {
        let out = rc(&work, &target, args);
        assert!(out.status.success(), "rc {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.lines().next().and_then(|l| l.strip_prefix('#')).and_then(|l| l.split_whitespace().next()).expect("an id").to_owned()
    };
    let project = id_of(&["add", "project", "Alpha"]);
    let milestone = id_of(&["add", "milestone", "M", "--parent", &project]);
    let card = id_of(&["add", "task", "T", "--parent", &milestone, "--acceptance", "`cargo test it_runs`"]);
    commit(&work, "shared", "base\n");
    git(&work, &["checkout", "-q", "-b", "card"]);
    (dir, work, target, card)
}

#[test]
fn a_check_merges_main_into_a_branch_that_is_behind() {
    let (_dir, work, target, card) = project();
    git(&work, &["checkout", "-q", "main"]);
    commit(&work, "from_main", "fixed\n");
    git(&work, &["checkout", "-q", "card"]);
    assert!(!work.join("from_main").exists());

    let out = rc(&work, &target, &["check", &card]);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(out.status.code(), Some(0), "{stdout}{}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout.contains("brought main in"), "{stdout}");
    assert!(work.join("from_main").exists());

    let again = String::from_utf8_lossy(&rc(&work, &target, &["check", &card]).stdout).into_owned();
    assert!(!again.contains("brought main in"), "{again}");
}

#[test]
fn a_merge_that_conflicts_is_a_conflict_and_not_a_failed_check() {
    let (_dir, work, target, card) = project();
    git(&work, &["checkout", "-q", "main"]);
    commit(&work, "shared", "main's\n");
    git(&work, &["checkout", "-q", "card"]);
    commit(&work, "shared", "the card's\n");

    let out = rc(&work, &target, &["check", &card]);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(out.status.code(), Some(1), "{stdout}");
    assert!(stdout.contains("conflict") && stdout.contains("shared"), "{stdout}");
    assert!(!stdout.contains("failed") && !stdout.contains("passed"), "no check ran: {stdout}");
    assert_eq!(std::fs::read_to_string(work.join("shared")).unwrap(), "the card's\n", "the tree is as it was");
    assert!(!work.join(".git").join("MERGE_HEAD").exists(), "the merge was aborted");
}
