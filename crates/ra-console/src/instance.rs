//! One console at a time, and a way to tell the running one to leave without a word to any
//! model. The console claims `var/console.pid` while it runs and polls for `var/console.quit`
//! on its tick; a newcomer with `--replace` drops the quit file, waits for the pid file to go,
//! and kills the process when it does not. The launch script does the same from PowerShell, so
//! a rebuild never finds the binary locked by a console that is still up.

use std::path::Path;
use std::time::{Duration, Instant};

/// Where the running console leaves its process id.
const PID_FILE: &str = "var/console.pid";
/// The file whose presence tells the running console to quit.
const QUIT_FILE: &str = "var/console.quit";
/// How long a console gets to quit on its own before it is killed.
const GRACE: Duration = Duration::from_secs(3);

/// Claims the instance: writes this process id. A stale file from a dead console is overwritten.
pub fn claim() -> anyhow::Result<()> {
    std::fs::create_dir_all("var")?;
    std::fs::write(PID_FILE, std::process::id().to_string())?;
    let _ = std::fs::remove_file(QUIT_FILE);
    Ok(())
}

/// Releases the instance: removes the pid file, if it still names this process.
pub fn release() {
    if running_pid() == Some(std::process::id()) {
        let _ = std::fs::remove_file(PID_FILE);
    }
}

/// Whether another process asked this console to quit; the request is consumed.
pub fn quit_requested() -> bool {
    Path::new(QUIT_FILE).exists() && std::fs::remove_file(QUIT_FILE).is_ok()
}

/// The pid in the pid file, when there is one.
fn running_pid() -> Option<u32> {
    std::fs::read_to_string(PID_FILE).ok()?.trim().parse().ok()
}

/// Tells a running console to quit and waits for it; kills it after the grace period. Returns
/// what happened, for the log, or `None` when no console was running.
pub fn stop_running() -> Option<String> {
    let pid = running_pid()?;
    if pid == std::process::id() {
        return None;
    }
    if std::fs::write(QUIT_FILE, "").is_err() {
        return Some(format!("cannot ask console {pid} to quit"));
    }
    let deadline = Instant::now() + GRACE;
    while Instant::now() < deadline {
        if !Path::new(PID_FILE).exists() {
            let _ = std::fs::remove_file(QUIT_FILE);
            return Some(format!("console {pid} quit"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = std::fs::remove_file(QUIT_FILE);
    let killed = kill(pid);
    let _ = std::fs::remove_file(PID_FILE);
    Some(if killed { format!("console {pid} did not quit in time and was killed") } else { format!("console {pid} was already gone") })
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
