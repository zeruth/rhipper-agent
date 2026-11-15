//! A worker may run a binary its card built, and list the target it sits in, from either shell;
//! another card's target stays refused.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// The hook's answer to `call`, run from card 840's worktree under `base` with its scratch root.
fn pre(base: &Path, call: &serde_json::Value) -> String {
    let worktree = base.join("proj/var/worktrees/840");
    std::fs::create_dir_all(&worktree).expect("a worktree");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rc"))
        .args(["hook", "pre"])
        .current_dir(&worktree)
        .env("RHIPPER_SCRATCH", base.join("scratch"))
        // the run's own TEMP counts as the card's: keep the test's tempdir out of it
        .env("TEMP", base.join("t"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("rc runs");
    let mut call = call.clone();
    call["cwd"] = worktree.to_string_lossy().into_owned().into();
    child.stdin.take().expect("stdin").write_all(call.to_string().as_bytes()).expect("the event");
    let out = child.wait_with_output().expect("an answer");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn a_binary_under_the_cards_own_target_runs_and_another_cards_does_not() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let base = dir.path();
    let own = base.join("scratch/target/proj/wt-840").to_string_lossy().replace('\\', "/");
    let other = base.join("scratch/target/proj/wt-839").to_string_lossy().replace('\\', "/");
    let shell = |tool: &str, command: String| serde_json::json!({"tool_name": tool, "tool_input": {"command": command}});
    for (tool, command) in [
        ("PowerShell", format!("& \"{own}/debug/ras.exe\" lift")),
        ("PowerShell", format!("& {own}/debug/ras.exe lift")),
        ("PowerShell", format!("Get-ChildItem {own}/debug")),
        ("Bash", format!("{own}/debug/ras.exe lift")),
        ("Bash", format!("ls {own}/debug")),
    ] {
        let answer = pre(base, &shell(tool, command.clone()));
        assert!(answer.contains("\"allow\""), "{tool} {command}: {answer}");
    }
    // another card's target may be listed (a read-only root) but not run from
    let command = format!("& \"{other}/debug/ras.exe\" lift");
    let answer = pre(base, &shell("PowerShell", command.clone()));
    assert!(!answer.contains("\"allow\""), "{command}: {answer}");
}
