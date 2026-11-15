//! A card's catch-up merges only the work the project has kept: a squash that another card's
//! landing put on the project's branch and is still proving with its check stays off the
//! card's branch, which then never holds that card's commit after the check fails.

use std::path::Path;

use ra_console::agent::{PROVING_REF, catch_up_target, merge_main_in};

fn git_ok(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git").args(args).current_dir(dir).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn commit(dir: &Path, file: &str, text: &str, message: &str) {
    std::fs::write(dir.join(file), text).unwrap();
    git_ok(dir, &["add", file]);
    git_ok(dir, &["commit", "-q", "-m", message]);
}

/// A project on `main` with a card's worktree on `card/2`, the project one commit ahead of it.
fn project(root: &Path) -> std::path::PathBuf {
    git_ok(root, &["init", "-q", "-b", "main"]);
    git_ok(root, &["config", "user.name", "test"]);
    git_ok(root, &["config", "user.email", "test@example.com"]);
    commit(root, "base.rs", "base\n", "Base");
    let tree = root.join("wt");
    git_ok(root, &["worktree", "add", "-q", "-b", "card/2", &tree.to_string_lossy()]);
    commit(&tree, "own.rs", "own\n", "The card's own work");
    commit(root, "kept.rs", "kept\n", "Work the project kept");
    tree
}

fn files_of_branch(tree: &Path) -> String {
    git_ok(tree, &["ls-tree", "-r", "--name-only", "HEAD"])
}

#[tokio::test]
async fn a_catch_up_during_another_landings_check_leaves_its_squash_out() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let tree = project(root);
    let kept = git_ok(root, &["rev-parse", "HEAD"]);
    // another card's landing: its squash on main, under its merged check
    commit(root, "landing_of_another_card.rs", "theirs\n", "Another card's change");
    git_ok(root, &["update-ref", PROVING_REF, "HEAD"]);

    let target = catch_up_target(root, "main").await;
    assert_eq!(target, kept, "the catch-up merges the commit before the squash under proof");
    assert_eq!(merge_main_in(&tree, &target).await, None);
    let files = files_of_branch(&tree);
    assert!(files.contains("kept.rs") && files.contains("own.rs"), "{files}");
    assert!(!files.contains("landing_of_another_card.rs"), "the unproven squash reached the card's branch: {files}");

    // the check failed: the squash leaves main, and the card's branch never had it
    git_ok(root, &["reset", "-q", "--merge", "HEAD~1"]);
    git_ok(root, &["update-ref", "-d", PROVING_REF]);
    let log = git_ok(&tree, &["log", "--format=%s", "card/2"]);
    assert!(!log.contains("Another card's change"), "{log}");
}

#[tokio::test]
async fn without_a_landing_under_check_the_catch_up_takes_main() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let _tree = project(root);
    assert_eq!(catch_up_target(root, "main").await, "main");
}

#[tokio::test]
async fn a_proving_ref_whose_squash_main_dropped_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let tree = project(root);
    commit(root, "landing_of_another_card.rs", "theirs\n", "Another card's change");
    git_ok(root, &["update-ref", PROVING_REF, "HEAD"]);
    git_ok(root, &["reset", "-q", "--merge", "HEAD~1"]);
    assert_eq!(catch_up_target(root, "main").await, "main");
    assert_eq!(merge_main_in(&tree, "main").await, None);
    assert!(!files_of_branch(&tree).contains("landing_of_another_card.rs"));
}
