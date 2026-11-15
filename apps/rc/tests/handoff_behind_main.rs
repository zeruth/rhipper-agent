//! A rerun on a tree behind main says how many commits it lacks, names the subjects of those
//! that touch the card's files or their crates, and tells the session to merge main first.

use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git").args(args).current_dir(dir).output().expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn commit(dir: &Path, name: &str, subject: &str) {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).expect("a directory");
    std::fs::write(&path, subject).expect("a file");
    git(dir, &["add", "."]);
    git(dir, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", subject]);
}

#[test]
fn a_tree_behind_main_names_the_commits_that_touch_the_card() {
    let dir = tempfile::tempdir().expect("a tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    commit(dir.path(), "crates/x/src/a.rs", "start");
    git(dir.path(), &["checkout", "-q", "-b", "card"]);
    commit(dir.path(), "crates/x/src/own.rs", "card work");
    git(dir.path(), &["checkout", "-q", "main"]);
    commit(dir.path(), "crates/x/src/b.rs", "change in the crate");
    commit(dir.path(), "crates/y/src/c.rs", "change elsewhere");
    git(dir.path(), &["checkout", "-q", "card"]);

    let lines = ra_console::agent::behind_main(dir.path(), &["crates/x/src/a.rs".to_owned()]);
    let text = lines.join("\n");
    assert!(text.contains("2 commits behind main"), "{text}");
    assert!(text.contains("merge main first"), "{text}");
    assert!(text.contains("change in the crate"), "{text}");
    assert!(!text.contains("change elsewhere"), "{text}");
}

#[test]
fn a_tree_level_with_main_says_nothing() {
    let dir = tempfile::tempdir().expect("a tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    commit(dir.path(), "a.txt", "start");
    assert!(ra_console::agent::behind_main(dir.path(), &["a.txt".to_owned()]).is_empty());
}
