//! `rc wait` on a job whose process is alive does not give up at its timeout (nine minutes in a
//! worker's turn): it waits for the exit line. The timeout here is one second, the same path.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn rc_wait_keeps_waiting_on_a_job_that_still_runs() {
    let dir = std::env::temp_dir().join(format!("rc-wait-outlives-{}", std::process::id()));
    let jobs = dir.join("var").join("jobs");
    std::fs::create_dir_all(&jobs).expect("a jobs dir");
    // This test process is the job's live process: its id is the job's id.
    let id = std::process::id();
    std::fs::write(jobs.join(format!("{id}.job")), "cargo build").expect("a job record");

    let started = Instant::now();
    let waiter = Command::new(env!("CARGO_BIN_EXE_rc"))
        .args(["wait", &id.to_string(), "--timeout", "1"])
        .current_dir(&dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("rc wait");

    std::thread::sleep(Duration::from_secs(4));
    std::fs::write(jobs.join(format!("{id}.exit")), format!("{id} exited 7\n")).expect("an exit line");

    let out = waiter.wait_with_output().expect("rc wait ends");
    assert_eq!(out.status.code(), Some(7), "{}", String::from_utf8_lossy(&out.stdout));
    assert!(started.elapsed() >= Duration::from_secs(4), "gave up after {:?}", started.elapsed());
    let _ = std::fs::remove_dir_all(&dir);
}
