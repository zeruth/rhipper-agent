//! The finished state of a background job, read before `rc wait` waits on anything and at every
//! poll of the wait: a build that ended while it was queued behind others, or while the wait was
//! already on, has its exit line, and the wait answers with it.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How often a waited-on pid's exit line is looked for.
const POLL: Duration = Duration::from_millis(500);

fn exit_path(dir: &Path, id: u32) -> PathBuf {
    dir.join("var").join("jobs").join(format!("{id}.exit"))
}

/// The exit line `rc hold` writes for job `id`, the one [`finished`] reads back.
#[must_use]
pub(crate) fn exit_line(id: u32, code: i32) -> String {
    format!("{id} exited {code}\n")
}

/// The code an exit line of job `id` holds, `None` when the line is not one [`exit_line`] wrote.
#[must_use]
pub(crate) fn exit_code(id: u32, line: &str) -> Option<i32> {
    line.trim().strip_prefix(&format!("{id} exited "))?.trim().parse().ok()
}

/// The exit code of job `id` in `dir` when its exit line is written, `None` while it has none.
/// Prints the line, as the wait itself does; a line that holds no code answers 1, not a success.
#[must_use]
pub(crate) fn finished(dir: &Path, id: u32) -> Option<i32> {
    let line = std::fs::read_to_string(exit_path(dir, id)).ok()?;
    print!("{line}");
    // the job record is written when the build starts, so its age is the build's
    let started = std::fs::metadata(dir.join("var").join("jobs").join(format!("{id}.job"))).and_then(|m| m.modified());
    if let Ok(started) = started
        && let Some(note) = crate::stale_edits::note(&crate::stale_edits::edited_since(dir, started))
    {
        println!("{note}");
    }
    Some(exit_code(id, &line).unwrap_or_else(|| {
        eprintln!("rc wait: the exit line of {id} holds no exit code");
        1
    }))
}

/// Waits on pid `id` that has no job record yet, a build queued behind others: answers its exit
/// line's code as soon as one is written, 0 when the pid ends without one, 124 past `limit`.
#[must_use]
pub(crate) fn wait_pid(dir: &Path, id: u32, limit: Duration) -> i32 {
    let started = Instant::now();
    loop {
        if let Some(code) = finished(dir, id) {
            return code;
        }
        if !ra_console::hub::pid_alive(id) {
            // the exit line may land between the read and the pid's end
            return finished(dir, id).unwrap_or_else(|| {
                println!("rc wait: pid {id} has ended");
                0
            });
        }
        if started.elapsed() >= limit {
            eprintln!("rc wait: pid {id} still runs after {} s", limit.as_secs());
            return 124;
        }
        std::thread::sleep(POLL);
    }
}
