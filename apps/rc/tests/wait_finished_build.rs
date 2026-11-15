//! `rc wait` answers at once for a build whose exit line is already written.

use std::process::Command;
use std::time::{Duration, Instant};

fn project_with_exit(id: u32, code: i32, with_job: bool) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let jobs = dir.path().join("var").join("jobs");
    std::fs::create_dir_all(&jobs).unwrap();
    if with_job {
        std::fs::write(jobs.join(format!("{id}.job")), "cargo build").unwrap();
    }
    std::fs::write(jobs.join(format!("{id}.exit")), format!("{id} exited {code}\n")).unwrap();
    dir
}

fn wait(dir: &std::path::Path, id: u32) -> (i32, Duration) {
    let started = Instant::now();
    let status = Command::new(env!("CARGO_BIN_EXE_rc")).args(["wait", &id.to_string()]).current_dir(dir).output().unwrap().status;
    (status.code().unwrap(), started.elapsed())
}

#[test]
fn a_finished_build_is_answered_at_once() {
    let dir = project_with_exit(4_000_001, 3, true);
    let (code, took) = wait(dir.path(), 4_000_001);
    assert_eq!(code, 3);
    assert!(took < Duration::from_secs(20), "{took:?}");
}

#[test]
fn a_finished_build_without_its_job_record_is_answered_at_once() {
    let dir = project_with_exit(4_000_002, 0, false);
    let (code, took) = wait(dir.path(), 4_000_002);
    assert_eq!(code, 0);
    assert!(took < Duration::from_secs(20), "{took:?}");
}

#[test]
fn an_exit_line_without_a_code_is_no_success() {
    let dir = tempfile::tempdir().unwrap();
    let jobs = dir.path().join("var").join("jobs");
    std::fs::create_dir_all(&jobs).unwrap();
    std::fs::write(jobs.join("4000003.exit"), "4000003 exited\n").unwrap();
    let (code, _) = wait(dir.path(), 4_000_003);
    assert_eq!(code, 1);
}

/// Waits on this test's own pid, alive the whole wait, while a thread plays a build queued
/// behind others: its record lands after `record` when given, its exit line after `exit`.
fn wait_on_a_queued_build(record: Option<Duration>, exit: Duration, code: i32) -> (i32, Duration) {
    let dir = tempfile::tempdir().unwrap();
    let jobs = dir.path().join("var").join("jobs");
    std::fs::create_dir_all(&jobs).unwrap();
    let id = std::process::id();
    let builder = {
        let jobs = jobs.clone();
        std::thread::spawn(move || {
            let started = Instant::now();
            if let Some(record) = record {
                std::thread::sleep(record);
                std::fs::write(jobs.join(format!("{id}.job")), "cargo build").unwrap();
            }
            std::thread::sleep(exit.saturating_sub(started.elapsed()));
            std::fs::write(jobs.join(format!("{id}.exit")), format!("{id} exited {code}\n")).unwrap();
        })
    };
    let answer = wait(dir.path(), id);
    builder.join().unwrap();
    answer
}

#[test]
fn a_build_that_ends_during_the_wait_is_answered_when_it_ends() {
    let (code, took) = wait_on_a_queued_build(None, Duration::from_secs(2), 5);
    assert_eq!(code, 5);
    assert!(took >= Duration::from_millis(1500) && took < Duration::from_secs(20), "{took:?}");
}

#[test]
fn a_queued_build_recorded_during_the_wait_is_answered_when_it_ends() {
    let (code, took) = wait_on_a_queued_build(Some(Duration::from_secs(1)), Duration::from_secs(3), 2);
    assert_eq!(code, 2);
    assert!(took >= Duration::from_millis(2500) && took < Duration::from_secs(20), "{took:?}");
}
