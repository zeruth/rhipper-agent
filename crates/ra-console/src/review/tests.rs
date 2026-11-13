//! The tests of the review's scope and of the head of a handoff, on a scratch git repository.

use std::path::Path;

use super::*;

fn git_ok(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(["-c", "user.name=test", "-c", "user.email=test@example.com"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn commit(dir: &Path, file: &str, text: &str, message: &str) {
    std::fs::write(dir.join(file), text).unwrap();
    git_ok(dir, &["add", "-A"]);
    git_ok(dir, &["commit", "-q", "-m", message]);
}

#[tokio::test]
async fn review_scope_covers_two_runs_commits_and_an_untracked_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_ok(root, &["init", "-q"]);
    commit(root, "card.rs", "fn card() {}\n", "base");
    let base = git_ok(root, &["rev-parse", "HEAD"]);
    // two runs, each checkpointed on the branch, then a third that left a new file untracked
    commit(root, "first.rs", "fn first() {}\n", "Checkpoint: run 1");
    commit(root, "second.rs", "fn second() {}\n", "Checkpoint: run 2");
    std::fs::write(root.join("third.rs"), "fn third() {}\n").unwrap();
    let named = vec!["card.rs".to_owned()];
    let files = scope(root, &base, true, &named).await;
    assert_eq!(files, ["card.rs", "first.rs", "second.rs", "third.rs"]);
    let stat = diff_stat_section(root, &base, &files).await;
    assert!(stat.contains("first.rs") && stat.contains("second.rs"), "{stat}");
    // on the project's shared tree only the named paths count
    assert_eq!(scope(root, "HEAD", false, &named).await, ["card.rs"]);
    let section = files_section(&named, &files, &[]);
    assert!(section.contains("- first.rs (changed by the card's runs, not on the card)"), "{section}");
}

#[tokio::test]
async fn review_scope_handoff_head_says_the_tree_was_merged_since() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_ok(root, &["init", "-q"]);
    commit(root, "card.rs", "fn card() {}\n", "base");
    let base = git_ok(root, &["rev-parse", "HEAD"]);
    git_ok(root, &["checkout", "-q", "-b", "card/7"]);
    commit(root, "card.rs", "fn card() { 1 }\n", "Checkpoint: run 1");
    let tip = git_ok(root, &["rev-parse", "--short=12", "HEAD"]);
    // the project moved on and was merged into the card's branch between two runs
    git_ok(root, &["checkout", "-q", "-b", "project", &base]);
    commit(root, "other.rs", "fn other() {}\n", "another card landed");
    git_ok(root, &["checkout", "-q", "card/7"]);
    git_ok(root, &["merge", "-q", "--no-ff", "--no-edit", "project"]);
    let since = since_last(root, &tip).await;
    assert!(since.as_deref().is_some_and(|s| s.starts_with("merged: Merge branch 'project'")), "{since:?}");
    let tree = tree_of(root, &base, Some("card/7"), since).await;
    let head = tree.head(&["cargo test -p x".to_owned()]);
    assert!(head.starts_with(&format!("tree: at {}", tree.tip)), "{head}");
    assert!(head.contains("on card/7 · 3 commit(s) on the branch since its base"), "{head}");
    assert!(head.contains("\nsince the last run: merged: "), "{head}");
    assert!(head.ends_with("open: `cargo test -p x`\n"), "{head}");
    // where the run left it, nothing happened since
    let now = git_ok(root, &["rev-parse", "--short=12", "HEAD"]);
    assert_eq!(since_last(root, &now).await, None);
}
