//! A read-only `grep` or `tail` of the run's own background task output, which lives under the
//! run's scratch directory, passes the hook; another run's file stays refused.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// The hook's answer to a Bash `command`, run from card 807's worktree under `base`.
fn pre(base: &Path, command: &str) -> String {
    pre_in(base, &base.join("scratch"), command)
}

/// [`pre`] with the scratch root at `scratch`.
fn pre_in(base: &Path, scratch: &Path, command: &str) -> String {
    let worktree = base.join("proj/var/worktrees/807");
    std::fs::create_dir_all(&worktree).expect("a worktree");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rc"))
        .args(["hook", "pre"])
        .current_dir(&worktree)
        .env("RHIPPER_SCRATCH", scratch)
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("TEMP")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("rc runs");
    let call = serde_json::json!({"tool_name": "Bash", "tool_input": {"command": command}, "cwd": worktree.to_string_lossy()});
    child.stdin.take().expect("stdin").write_all(call.to_string().as_bytes()).expect("the event");
    String::from_utf8_lossy(&child.wait_with_output().expect("an answer").stdout).into_owned()
}

fn output_of(base: &Path, run: u32) -> String {
    base.join(format!("scratch/tmp/proj/run-{run}/claude/abc/def/tasks/bpgsrb4bu.output")).to_string_lossy().replace('\\', "/")
}

#[test]
fn grep_and_tail_of_the_runs_own_task_output_are_allowed() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let file = output_of(dir.path(), 807);
    for command in [format!("grep -n error {file}"), format!("tail -n 40 {file}"), format!("tail {file}")] {
        let answer = pre(dir.path(), &command);
        assert!(answer.contains("\"allow\""), "{command}: {answer}");
    }
}

/// A scratch root on another drive is what the outside-path gotcha names.
#[test]
fn a_scratch_root_on_the_d_drive_passes_for_the_runs_own_file_only() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let scratch = Path::new("D:/rhipper-hook-task-output");
    let file = |run: u32| format!("D:/rhipper-hook-task-output/tmp/proj/run-{run}/claude/a/b/tasks/x.output");
    for command in [format!("grep -n error {}", file(807)), format!("tail -n 40 {}", file(807))] {
        let answer = pre_in(dir.path(), scratch, &command);
        assert!(answer.contains("\"allow\""), "{command}: {answer}");
    }
    let command = format!("tail -n 40 {}", file(806));
    assert!(!pre_in(dir.path(), scratch, &command).contains("\"allow\""));
}

#[test]
fn grep_and_tail_of_another_runs_task_output_are_refused() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let file = output_of(dir.path(), 806);
    for command in [format!("grep -n error {file}"), format!("tail -n 40 {file}")] {
        let answer = pre(dir.path(), &command);
        assert!(!answer.contains("\"allow\""), "{command}: {answer}");
    }
}
