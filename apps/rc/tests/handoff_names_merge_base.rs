//! A card's handoff names the SHA its branch forked from and the `git diff` that uses it, so a
//! worker never writes the `$(git merge-base ..)` the hook refuses or `git diff main HEAD`.

use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

#[test]
fn the_handoff_head_prints_the_merge_base_and_its_diff_command() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let root = dir.path();
    git(root, &["init", "-q"]);
    std::fs::write(root.join("a.txt"), "one").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "base"]);
    let fork = git(root, &["rev-parse", "HEAD"]);
    git(root, &["checkout", "-q", "-b", "card/1"]);
    std::fs::write(root.join("b.txt"), "two").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "work"]);

    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let tree = runtime.block_on(ra_console::review::tree_of(root, &fork, Some("card/1"), None));
    let head = tree.head(&[]);

    let line = format!("`git diff {fork} HEAD --stat`");
    assert!(head.contains(&line), "{head}");
}
