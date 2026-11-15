use std::path::Path;

use ra_console::agent::branch_files;

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

fn commit(dir: &Path, file: &str, text: &str) {
    std::fs::write(dir.join(file), text).unwrap();
    git_ok(dir, &["add", "-A"]);
    git_ok(dir, &["commit", "-q", "-m", text]);
}

#[tokio::test]
async fn a_merge_of_main_adds_nothing_to_the_cards_changed_files() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    git_ok(&project, &["init", "-q", "-b", "main"]);
    commit(&project, "card.rs", "base");
    commit(&project, "shared.rs", "a\nb\nc\nd\ne\n");
    let tree = dir.path().join("card");
    git_ok(&project, &["worktree", "add", "-q", "-b", "card/7", tree.to_str().unwrap()]);
    commit(&tree, "card.rs", "card");
    commit(&tree, "shared.rs", "A\nb\nc\nd\ne\n");
    commit(&project, "other.rs", "main moved");

    // merged and committed: the merge base is main's tip
    git_ok(&tree, &["merge", "-q", "--no-edit", "main"]);
    assert_eq!(branch_files(&project, &tree).await, vec!["card.rs", "shared.rs"]);

    // merged and waiting for its commit, as a run capped before its checkpoint leaves it; main
    // also changed a file the card changed, which stays the card's
    commit(&project, "later.rs", "main moved again");
    commit(&project, "shared.rs", "a\nb\nc\nd\nE\n");
    git_ok(&tree, &["merge", "-q", "--no-commit", "--no-ff", "main"]);
    assert!(git_ok(&tree, &["status", "--short"]).contains("later.rs"));
    assert_eq!(branch_files(&project, &tree).await, vec!["card.rs", "shared.rs"]);

    // main moved on past the pending merge: its incoming tip is still the base
    commit(&project, "after.rs", "main moved during the merge");
    assert_eq!(branch_files(&project, &tree).await, vec!["card.rs", "shared.rs"]);
    git_ok(&tree, &["commit", "-q", "--no-edit"]);

    // a pending merge of a branch that is not main's brings the card's own files
    git_ok(&tree, &["checkout", "-q", "-b", "side"]);
    commit(&tree, "side.rs", "side");
    git_ok(&tree, &["checkout", "-q", "card/7"]);
    git_ok(&tree, &["merge", "-q", "--no-commit", "--no-ff", "side"]);
    assert_eq!(branch_files(&project, &tree).await, vec!["card.rs", "shared.rs", "side.rs"]);
}
