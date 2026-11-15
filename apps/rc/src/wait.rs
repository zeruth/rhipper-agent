//! `rc wait` on the background jobs of a run: `rc hold` records every command it runs under
//! `var/jobs/`, and a wait names one of them, or the newest, and blocks until its exit line.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How often a waiting job's exit line is looked for.
const POLL: Duration = Duration::from_millis(500);

/// How often a job that still runs says so.
const PROGRESS: Duration = Duration::from_mins(1);

fn jobs_dir(dir: &Path) -> PathBuf {
    dir.join("var").join("jobs")
}

/// A command running under `rc hold`: its record is written on start and its exit line when it
/// ends, so a wait on its id has something to read.
pub(crate) struct Job {
    dir: PathBuf,
    id: u32,
}

impl Job {
    /// Records the running command as a job of this process; a directory that cannot be written
    /// is nobody's failure, the command runs unrecorded.
    #[must_use]
    pub(crate) fn start(dir: &Path, command: &str) -> Self {
        let id = std::process::id();
        let jobs = jobs_dir(dir);
        let _ = std::fs::create_dir_all(&jobs);
        let _ = std::fs::remove_file(jobs.join(format!("{id}.exit")));
        let _ = std::fs::write(jobs.join(format!("{id}.job")), command);
        Self { dir: dir.to_path_buf(), id }
    }

    /// Writes the exit line the waiters read.
    pub(crate) fn finish(&self, code: i32) {
        let _ = std::fs::write(jobs_dir(&self.dir).join(format!("{}.exit", self.id)), crate::wait_finished::exit_line(self.id, code));
    }
}

/// The id of the job most recently started in `dir`.
fn newest(dir: &Path) -> Option<u32> {
    std::fs::read_dir(jobs_dir(dir))
        .ok()?
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|e| e == "job"))
        .filter_map(|entry| {
            let id = entry.path().file_stem()?.to_str()?.parse::<u32>().ok()?;
            Some((entry.metadata().ok()?.modified().ok()?, id))
        })
        .max()
        .map(|(_, id)| id)
}

/// Waits for a job of `dir` for `rc wait`: `target` is its id, the newest job when left out; a
/// target that is no job is a pid or a file, waited on as before under the `timeout`. A job is
/// waited on until its exit line however long it runs, with a progress line a minute. Answers the
/// job's own exit code, 1 when there is no job to wait for.
#[must_use]
pub(crate) fn run(dir: &Path, target: Option<&str>, timeout: Option<Duration>) -> i32 {
    let id = if let Some(t) = target {
        match t.trim().parse::<u32>() {
            // a build that ended while queued is answered at once, whatever its record says
            Ok(id) if jobs_dir(dir).join(format!("{id}.exit")).exists() => return crate::wait_finished::finished(dir, id).unwrap_or(1),
            Ok(id) if jobs_dir(dir).join(format!("{id}.job")).exists() => id,
            // a pid whose job is not recorded yet is still watched for its exit line
            Ok(id) => return crate::wait_finished::wait_pid(dir, id, timeout.unwrap_or(ra_console::agent::WAIT_LIMIT)),
            Err(_) => return ra_console::agent::wait(dir, t, timeout.unwrap_or(ra_console::agent::WAIT_LIMIT)),
        }
    } else {
        let Some(id) = newest(dir) else {
            eprintln!("rc wait: no background job has run here; name a pid or a file");
            return 1;
        };
        id
    };
    wait_job(dir, id, PROGRESS)
}

/// Waits for the exit line of job `id`, a line of progress every `progress`: a job that still runs
/// is waited on whatever the limit, a compile is not a silence.
fn wait_job(dir: &Path, id: u32, progress: Duration) -> i32 {
    println!("waiting {id}");
    let started = Instant::now();
    let mut last_line = started;
    loop {
        if let Some(code) = crate::wait_finished::finished(dir, id) {
            return code;
        }
        if !ra_console::hub::pid_alive(id) {
            // the exit line may land between the read and the pid's end
            return crate::wait_finished::finished(dir, id).unwrap_or_else(|| {
                println!("{id} ended without an exit line");
                1
            });
        }
        if last_line.elapsed() >= progress {
            println!("still running {id} · {}s", started.elapsed().as_secs());
            last_line = Instant::now();
        }
        std::thread::sleep(POLL);
    }
}
