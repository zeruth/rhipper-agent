use std::path::Path;
use std::process::Command;

use ra_console::agent::{check_is_stale, content_tip, stamped};

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@t"]).args(args).current_dir(dir).status().unwrap();
    assert!(status.success(), "git {args:?}");
}

#[tokio::test]
async fn a_check_stamped_on_an_older_tree_is_stale_once_the_branch_moved() {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "start"]);

    let before = content_tip(dir.path()).await.unwrap();
    let output = stamped("$ cargo test · exit code 0\nrunning 0 tests", Some(&before));
    assert!(!check_is_stale(&output, &before));

    // a commit of the same content (the walk's checkpoint) leaves the check current
    git(dir.path(), &["commit", "-q", "--allow-empty", "-m", "checkpoint"]);
    let same = content_tip(dir.path()).await.unwrap();
    assert!(!check_is_stale(&output, &same));

    // the test committed after the failed run moves it
    std::fs::write(dir.path().join("b.rs"), "fn b() {}\n").unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "the test"]);
    let moved = content_tip(dir.path()).await.unwrap();
    assert!(check_is_stale(&output, &moved));

    // an output with no stamp is read as stale; no output at all is no check to rerun
    assert!(check_is_stale("running 0 tests", &moved));
    assert!(!check_is_stale("", &moved));
}
