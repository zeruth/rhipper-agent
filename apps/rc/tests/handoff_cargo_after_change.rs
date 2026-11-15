//! A `cargo` build in a worktree with nothing changed is refused, and the reason says to write
//! the change first.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git").args(args).current_dir(dir).status().unwrap();
    assert!(status.success(), "git {args:?}");
}

fn hook(dir: &Path, command: &str) -> String {
    let call = serde_json::json!({
        "tool_name": "Bash",
        "session_id": "s1",
        "cwd": dir.to_string_lossy(),
        "tool_input": {"command": command},
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_rc"))
        .args(["hook", "pre"])
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(call.to_string().as_bytes()).unwrap();
    String::from_utf8_lossy(&child.wait_with_output().unwrap().stdout).into_owned()
}

#[test]
fn cargo_in_an_unchanged_worktree_is_refused_with_write_the_change_first() {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    std::fs::write(dir.path().join("a.rs"), "a").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "x"]);
    let out = hook(dir.path(), "cargo build");
    assert!(out.contains("deny"), "{out}");
    assert!(out.contains("write the card's change first"), "{out}");
}
