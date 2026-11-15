//! `rc wait` that gives up on a task still running says the task notification arrives without
//! another call, and a second `rc wait` for the same task is a gotcha.

use std::process::{Command, Stdio};

use ra_gotchas::{Verdict, repeated_wait};

#[test]
fn wait_gives_up_naming_the_task_notification() {
    let dir = std::env::temp_dir().join(format!("rc-wait-gives-up-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a dir");
    // This test process is alive for the whole wait, and no job record names it.
    let id = std::process::id();
    let out = Command::new(env!("CARGO_BIN_EXE_rc"))
        .args(["wait", &id.to_string(), "--timeout", "1"])
        .current_dir(&dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("rc wait");
    let said = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert_eq!(out.status.code(), Some(124), "{said}");
    assert!(said.contains("alive") && said.contains("task notification"), "{said}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn wait_gives_up_then_a_second_wait_for_the_task_is_a_gotcha() {
    let earlier = vec![String::from("rc wait 4242")];
    assert!(matches!(repeated_wait("rc wait 4242", &earlier), Some(Verdict::Deny { id, .. }) if id == "repeated-wait"));
    assert!(repeated_wait("rc wait 4243", &earlier).is_none());
    assert!(repeated_wait("rc wait 4242", &[]).is_none());
    assert!(repeated_wait("cargo test", &earlier).is_none());
}
