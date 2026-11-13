//! `rc wait` on a running job blocks until its exit line however short its `--timeout`, and
//! prints the job's result once.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn rc_wait_holds_until_the_job_ends_and_prints_its_result_once() {
    let dir = std::env::temp_dir().join(format!("rc-wait-long-{}", std::process::id()));
    let jobs = dir.join("var").join("jobs");
    std::fs::create_dir_all(&jobs).expect("a jobs dir");
    // This test process is the job's live process: its id is the job's id.
    let id = std::process::id();
    std::fs::write(jobs.join(format!("{id}.job")), "cargo test").expect("a job record");

    let started = Instant::now();
    let waiter = Command::new(env!("CARGO_BIN_EXE_rc"))
        .args(["wait", &id.to_string(), "--timeout", "1"])
        .current_dir(&dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("rc wait");

    std::thread::sleep(Duration::from_secs(3));
    std::fs::write(jobs.join(format!("{id}.exit")), format!("{id} exited 0\n")).expect("an exit line");

    let out = waiter.wait_with_output().expect("rc wait ends");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(started.elapsed() >= Duration::from_secs(3), "gave up after {:?}", started.elapsed());
    assert_eq!(stdout.matches("exited 0").count(), 1, "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}
