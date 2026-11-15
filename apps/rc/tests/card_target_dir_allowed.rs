//! A worker may run, list and read what is under its own card's target and scratch directories,
//! which the path hook would otherwise refuse as outside the workspace; another card's stay refused.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// The hook's answer to `call`, run from card 784's worktree under `base` with its scratch root.
fn pre(base: &Path, call: &serde_json::Value) -> String {
    let worktree = base.join("proj/var/worktrees/784");
    std::fs::create_dir_all(&worktree).expect("a worktree");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rc"))
        .args(["hook", "pre"])
        .current_dir(&worktree)
        .env("RHIPPER_SCRATCH", base.join("scratch"))
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

fn slashed(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[test]
fn the_cards_own_target_and_scratch_are_allowed_and_another_cards_is_not() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let base = dir.path();
    let own = slashed(&base.join("scratch/target/proj/wt-784"));
    let other = slashed(&base.join("scratch/target/proj/wt-783"));
    let run = slashed(&base.join("scratch/tmp/proj/run-784"));
    let shell = |command: String| serde_json::json!({"tool_name": "Bash", "tool_input": {"command": command}});
    for command in [format!("{own}/debug/rc.exe --version"), format!("ls {own}/debug"), format!("find {own} -name x"), format!("ls {run}")]
    {
        let answer = pre(base, &shell(command.clone()));
        assert!(answer.contains("\"allow\""), "{command}: {answer}");
    }
    // a listing of another card's target is the project's target root, which #805 lets through;
    // running another card's binary stays refused
    let command = format!("{other}/debug/rc.exe --version");
    let answer = pre(base, &shell(command.clone()));
    assert!(!answer.contains("\"allow\""), "{command}: {answer}");
}

#[test]
fn the_read_tools_take_the_cards_own_directories_only() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let base = dir.path();
    let own = slashed(&base.join("scratch/target/proj/wt-784/debug/out.txt"));
    let other = slashed(&base.join("scratch/target/proj/wt-783/debug/out.txt"));
    let read = |path: &str| serde_json::json!({"tool_name": "Read", "tool_input": {"file_path": path}});
    assert!(pre(base, &read(&own)).contains("\"allow\""));
    assert!(!pre(base, &read(&other)).contains("\"allow\""));
    let glob =
        serde_json::json!({"tool_name": "Glob", "tool_input": {"pattern": "*", "path": slashed(&base.join("scratch/tmp/proj/run-784"))}});
    assert!(pre(base, &glob).contains("\"allow\""));
}
