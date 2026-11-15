use std::path::Path;

use ra_console::review::tree_of;

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

fn commit(dir: &Path, file: &str, message: &str) {
    std::fs::write(dir.join(file), message).unwrap();
    git_ok(dir, &["add", "-A"]);
    git_ok(dir, &["commit", "-q", "-m", message]);
}

#[tokio::test]
async fn a_branch_ahead_of_its_base_lists_its_commits_and_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_ok(root, &["init", "-q"]);
    commit(root, "base.rs", "base");
    let base = git_ok(root, &["rev-parse", "HEAD"]);
    git_ok(root, &["checkout", "-q", "-b", "card/7"]);

    // a first run has no commit on the branch, so no section
    let first = tree_of(root, &base, Some("card/7"), None).await;
    assert_eq!(first.already(), "");

    commit(root, "one.rs", "Checkpoint: run 1");
    commit(root, "two.rs", "Checkpoint: run 2");
    let tree = tree_of(root, &base, Some("card/7"), None).await;
    let section = tree.already();
    assert!(section.starts_with("Already on the branch:\n"), "{section}");
    assert!(section.contains(" Checkpoint: run 2\n"), "{section}");
    assert!(section.contains(" Checkpoint: run 1\n"), "{section}");
    assert!(section.contains("files: one.rs, two.rs\n"), "{section}");
}
