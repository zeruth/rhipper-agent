//! The hook lets a worker run a binary under its own card's target directory, in either shell and
//! with either slash; another card's binary stays refused.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// The hook's answer to `call`, run from card 810's worktree under `base` with its scratch root.
fn pre(base: &Path, call: &serde_json::Value) -> String {
    let worktree = base.join("proj/var/worktrees/810");
    std::fs::create_dir_all(&worktree).expect("a worktree");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rc"))
        .args(["hook", "pre"])
        .current_dir(&worktree)
        .env("RHIPPER_SCRATCH", base.join("scratch"))
        // a run's own TEMP and target are roots of their own, and the tempdir lies under them
        .env_remove("TEMP")
        .env_remove("TMP")
        .env_remove("CARGO_TARGET_DIR")
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

fn run(base: &Path, tool: &str, command: &str) -> String {
    pre(base, &serde_json::json!({"tool_name": tool, "tool_input": {"command": command}}))
}

#[test]
fn the_cards_own_binary_is_allowed_and_another_cards_is_refused() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let base = dir.path();
    let target = base.join("scratch/target/proj");
    let forms = |card: u32| {
        let exe = target.join(format!("wt-{card}")).join("debug").join("ras.exe");
        let back = exe.to_string_lossy().replace('/', "\\");
        let slashed = back.replace('\\', "/");
        vec![
            ("Bash", format!("{slashed} emit x.bin")),
            ("Bash", format!("\"{back}\" emit x.bin")),
            ("PowerShell", format!("& \"{back}\" emit x.bin")),
            ("PowerShell", format!("& '{back}' emit x.bin")),
            ("PowerShell", format!("{back} emit x.bin")),
        ]
    };
    for (tool, command) in forms(810) {
        let answer = run(base, tool, &command);
        assert!(answer.contains("\"allow\""), "{tool}: {command}: {answer}");
    }
    for (tool, command) in forms(809) {
        let answer = run(base, tool, &command);
        assert!(!answer.contains("\"allow\""), "{tool}: {command}: {answer}");
    }
}
