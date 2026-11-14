//! A check after a merge from main rebuilds the card's own crates, so no stale rlib is linked; a
//! check with no merge in between builds incrementally.

use std::path::Path;
use std::process::{Command, Output};

use ra_providers::rebuild::{crates_of, rebuild_after_merge};

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(status.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&status.stderr));
}

fn commit(dir: &Path, file: &str) {
    std::fs::write(dir.join(file), file).unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", file]);
}

/// Main moves by a commit of `file` and is merged into the card's branch.
fn merge_main(dir: &Path, file: &str) {
    git(dir, &["checkout", "-q", "main"]);
    commit(dir, file);
    git(dir, &["checkout", "-q", "card"]);
    git(dir, &["merge", "-q", "--no-ff", "-m", "merge main", "main"]);
}

fn print(target: &Path, name: &str) -> std::path::PathBuf {
    target.join("debug").join(".fingerprint").join(name)
}

#[test]
fn a_check_rebuilds_the_cards_crates_after_a_merge_from_main() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let (work, target) = (dir.path().join("work"), dir.path().join("target"));
    std::fs::create_dir_all(&work).unwrap();
    git(&work, &["init", "-q", "-b", "main"]);
    commit(&work, "a");
    git(&work, &["checkout", "-q", "-b", "card"]);
    let commands = ["cargo test -p lift -q".to_owned(), "cargo test --package=lift --test x".to_owned()];
    assert_eq!(crates_of(&commands), ["lift"]);
    for name in ["lift-0123456789abcdef", "lift-extra-0123456789abcdef", "other-0123456789abcdef"] {
        std::fs::create_dir_all(print(&target, name)).unwrap();
    }
    let check = || rebuild_after_merge(&work, &target, &commands).unwrap();

    // the first check has nothing to compare with
    let first = check();
    assert!(first.cleaned.is_empty());
    first.seen().unwrap();
    // a commit of the card's own is no merge: the build stays incremental
    commit(&work, "b");
    let own = check();
    assert!(own.cleaned.is_empty());
    own.seen().unwrap();
    assert!(print(&target, "lift-0123456789abcdef").exists());

    merge_main(&work, "c");
    // a check interrupted before it ran records nothing: the next one cleans again
    assert_eq!(check().cleaned, ["lift"]);
    std::fs::create_dir_all(print(&target, "lift-0123456789abcdef")).unwrap();
    let merged = check();
    assert_eq!(merged.cleaned, ["lift"]);
    assert!(!print(&target, "lift-0123456789abcdef").exists());
    assert!(print(&target, "lift-extra-0123456789abcdef").exists(), "another crate's print stays");
    assert!(print(&target, "other-0123456789abcdef").exists());
    merged.seen().unwrap();

    // the same HEAD again is no new merge
    std::fs::create_dir_all(print(&target, "lift-0123456789abcdef")).unwrap();
    assert!(check().cleaned.is_empty());
    assert!(print(&target, "lift-0123456789abcdef").exists());
}

fn rc(dir: &Path, target: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rc"))
        .args(args)
        .env_remove("RHIPPER_SCRATCH")
        .env("CARGO_TARGET_DIR", target)
        .current_dir(dir)
        .output()
        .expect("rc runs")
}

#[test]
fn rc_check_rebuilds_the_workspace_after_a_merge_when_no_crate_is_named() {
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
    commit(&work, "a");
    git(&work, &["checkout", "-q", "-b", "card"]);
    let check = || {
        let out = rc(&work, &target, &["check", &card]);
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert_eq!(out.status.code(), Some(0), "{stdout}{}", String::from_utf8_lossy(&out.stderr));
        stdout
    };

    assert!(!check().contains("main was merged"));
    commit(&work, "b");
    assert!(!check().contains("main was merged"), "a commit of the card's own is no merge");

    merge_main(&work, "c");
    std::fs::create_dir_all(print(&target, "probe-0123456789abcdef")).unwrap();
    let merged = check();
    assert!(merged.contains("main was merged since the last check: probe rebuild"), "{merged}");
    assert!(!print(&target, "probe-0123456789abcdef").exists());

    assert!(!check().contains("main was merged"), "the merge was seen by the check that ran");
}
