//! One console or window per project at a time, and a way to tell the running one to leave
//! without a word to any model. The instance claims `<project>/var/console.pid` while it runs
//! and polls for `<project>/var/console.quit` on its tick; a newcomer with `--replace` drops the
//! quit file, waits for the pid file to go, and kills the process when it does not. The launch
//! scripts do the same from PowerShell. Every path is under the project's own `var/`, so two
//! projects run two instances, and a window with many projects as tabs claims each one.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Where the running instance leaves its process id, under the project.
const PID_FILE: &str = "var/console.pid";
/// The file whose presence tells the running instance to quit.
const QUIT_FILE: &str = "var/console.quit";
/// How long an instance gets to quit on its own before it is killed.
const GRACE: Duration = Duration::from_secs(3);

fn pid_file(root: &Path) -> PathBuf {
    root.join(PID_FILE)
}

fn quit_file(root: &Path) -> PathBuf {
    root.join(QUIT_FILE)
}

/// Claims the project: writes this process id. A stale file from a dead instance is overwritten.
pub fn claim(root: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(root.join("var"))?;
    std::fs::write(pid_file(root), std::process::id().to_string())?;
    let _ = std::fs::remove_file(quit_file(root));
    Ok(())
}

/// Releases the project: removes the pid file, if it still names this process.
pub fn release(root: &Path) {
    if running_pid(root) == Some(std::process::id()) {
        let _ = std::fs::remove_file(pid_file(root));
    }
}

/// Whether another process asked this instance to quit the project; the request is consumed.
pub fn quit_requested(root: &Path) -> bool {
    let quit = quit_file(root);
    quit.exists() && std::fs::remove_file(quit).is_ok()
}

/// The pid in the project's pid file, when there is one.
fn running_pid(root: &Path) -> Option<u32> {
    std::fs::read_to_string(pid_file(root)).ok()?.trim().parse().ok()
}

/// Tells the instance on the project to quit and waits for it; kills it after the grace period.
/// Returns what happened, for the log, or `None` when none was running. A window with the
/// project as one tab of many closes the tab, not itself.
pub fn stop_running(root: &Path) -> Option<String> {
    let pid = running_pid(root)?;
    if pid == std::process::id() {
        return None;
    }
    if std::fs::write(quit_file(root), "").is_err() {
        return Some(format!("cannot ask instance {pid} to quit"));
    }
    let deadline = Instant::now() + GRACE;
    while Instant::now() < deadline {
        if !pid_file(root).exists() {
            let _ = std::fs::remove_file(quit_file(root));
            return Some(format!("instance {pid} quit"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = std::fs::remove_file(quit_file(root));
    let killed = kill(pid);
    let _ = std::fs::remove_file(pid_file(root));
    Some(if killed { format!("instance {pid} did not quit in time and was killed") } else { format!("instance {pid} was already gone") })
}

/// Kills a process by id, the platform's way.
fn kill(pid: u32) -> bool {
    let status = if cfg!(windows) {
        std::process::Command::new("taskkill").args(["/PID", &pid.to_string(), "/F", "/T"]).output()
    } else {
        std::process::Command::new("kill").args(["-9", &pid.to_string()]).output()
    };
    status.is_ok_and(|o| o.status.success())
}
