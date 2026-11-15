//! A continuing card's handoff lists the files its tree already adds or changes against main,
//! with line counts, so the session writes instead of rediscovering them.

use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn a_continuing_handoff_lists_the_files_the_tree_adds_with_line_counts() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let root = dir.path();
    git(root, &["init", "-q", "-b", "main"]);
    std::fs::write(root.join("a.txt"), "one\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "base"]);
    git(root, &["checkout", "-q", "-b", "card/1"]);
    std::fs::write(root.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "work"]);
    std::fs::create_dir(root.join("sub")).unwrap();
    std::fs::write(root.join("sub").join("b.txt"), "x\ny\n").unwrap();

    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let note = runtime.block_on(ra_console::agent::prior_files_note(root, "main")).expect("a note");

    assert!(note.contains("`a.txt` +2 -0"), "{note}");
    assert!(note.contains("`sub/b.txt` new, 2 line(s)"), "{note}");
}

#[test]
fn a_tree_equal_to_main_has_no_note() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let root = dir.path();
    git(root, &["init", "-q", "-b", "main"]);
    std::fs::write(root.join("a.txt"), "one\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "base"]);

    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    assert!(runtime.block_on(ra_console::agent::prior_files_note(root, "main")).is_none());
}
