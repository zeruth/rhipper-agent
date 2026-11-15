//! `rc wait` refuses at once an id that is no task of the agent.

use std::process::Command;
use std::time::{Duration, Instant};

#[test]
fn an_unknown_wait_id_is_refused_at_once_with_the_notification_hint() {
    let dir = tempfile::tempdir().unwrap();
    let started = Instant::now();
    let out = Command::new(env!("CARGO_BIN_EXE_rc")).args(["wait", "bk3x9f2a1"]).current_dir(dir.path()).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(started.elapsed() < Duration::from_secs(20));
    assert!(String::from_utf8_lossy(&out.stderr).contains("task notification"));
}
