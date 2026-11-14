//! The look-back pass: every [`EVERY`] cards a project lands, its walk reads the workers' own
//! look-backs and the gotcha journal and plans the cards they call for (the evidence planner,
//! `rc plan`), then archives both logs so the next pass reads only what came after. The person's
//! loop of 2026-11-12, done by hand twice ("look at gotchas and look back, see what is new,
//! roadmap its worthwhile findings, and run again"), made the walk's own.
//!
//! The count of cards landed since the last pass and the pass's own marker live in the project's
//! `var`, so a restart neither loses the count nor runs a pass twice, and the window shows both.

use std::path::{Path, PathBuf};

/// How many landed cards make a look-back pass due.
pub const EVERY: u32 = 10;

/// The count of cards landed since the last pass.
const COUNT_FILE: &str = "var/lookback.count";

/// Present while a pass runs: the window lights its pill, and a second walk does not start one.
const RUNNING_FILE: &str = "var/lookback.running";

/// The last failed pass: its unix time and the landed count it failed at.
const FAILED_FILE: &str = "var/lookback.failed";

/// How long a failed pass waits when no more cards land.
const BACKOFF_SECS: u64 = 3600;

/// The logs a pass reads and then archives.
const LOGS: [&str; 2] = ["var/retro.log", "var/gotchas.log"];

/// The cards landed since the last pass.
#[must_use]
pub fn landed_since(root: &Path) -> u32 {
    std::fs::read_to_string(root.join(COUNT_FILE)).ok().and_then(|t| t.trim().parse().ok()).unwrap_or(0)
}

/// Counts one landed card toward the next pass.
pub fn count_landing(root: &Path) {
    let _ = std::fs::create_dir_all(root.join("var"));
    let _ = std::fs::write(root.join(COUNT_FILE), (landed_since(root) + 1).to_string());
}

/// Whether a pass is running now: its marker names a pid that is alive. A marker of a dead pass
/// (or one with no readable pid) holds nothing.
#[must_use]
pub fn running(root: &Path) -> bool {
    std::fs::read_to_string(root.join(RUNNING_FILE)).ok().and_then(|t| t.trim().parse::<u32>().ok()).is_some_and(crate::hub::pid_alive)
}

/// Whether a pass is due: [`EVERY`] cards landed and none running; after a failed pass, [`EVERY`]
/// more cards than it failed at, or [`BACKOFF_SECS`] later.
#[must_use]
pub fn due(root: &Path) -> bool {
    if landed_since(root) < EVERY || running(root) {
        return false;
    }
    let Some((at, landed)) = failure(root) else { return true };
    landed_since(root) >= landed + EVERY || now().saturating_sub(at) >= BACKOFF_SECS
}

fn now() -> u64 {
    crate::clock::unix_now()
}

/// The unix time and the landed count of the last failed pass, while it holds.
fn failure(root: &Path) -> Option<(u64, u32)> {
    let text = std::fs::read_to_string(root.join(FAILED_FILE)).ok()?;
    let (at, landed) = text.trim().split_once(' ')?;
    Some((at.parse().ok()?, landed.parse().ok()?))
}

/// The look-back pass: plans the cards the evidence calls for, archives the logs it read under
/// `var/archive/<time>/`, resets the count and says what it did on the feed. The marker is
/// taken first and always given back.
///
/// # Errors
/// When the planner fails; the logs and the count are then left for the next try.
pub async fn pass(store: &ra_roadmap::store::Store, worker: &crate::agent::Worker) -> anyhow::Result<Vec<String>> {
    let root = crate::agent::workspace_of(store);
    let marker = root.join(RUNNING_FILE);
    let _ = std::fs::write(&marker, std::process::id().to_string());
    let landed = landed_since(&root);
    let planned = crate::commands::plan::plan(store, worker, None, false).await;
    let _ = std::fs::remove_file(&marker);
    let lines = match planned {
        Ok(lines) => lines,
        Err(error) => {
            let _ = std::fs::write(root.join(FAILED_FILE), format!("{} {landed}", now()));
            crate::pair::event(&root, &format!("look-back pass failed; the next try waits for {EVERY} more landed cards or an hour"));
            return Err(error);
        }
    };
    let _ = std::fs::remove_file(root.join(FAILED_FILE));
    let archived = archive(&root);
    let _ = std::fs::write(root.join(COUNT_FILE), "0");
    let head = lines.first().cloned().unwrap_or_else(|| "nothing new to plan".to_owned());
    let text = format!("look-back pass after {landed} landed cards: {head}; {archived}");
    crate::pair::event(&root, &text);
    let mut out = vec![text];
    out.extend(lines);
    Ok(out)
}

/// Moves the logs a pass read to `var/archive/<unix time>/`; says what it moved.
fn archive(root: &Path) -> String {
    let now = now();
    let dir: PathBuf = root.join("var").join("archive").join(now.to_string());
    let mut moved = Vec::new();
    for log in LOGS {
        let from = root.join(log);
        if !from.is_file() {
            continue;
        }
        if std::fs::create_dir_all(&dir).is_ok()
            && let Some(name) = from.file_name()
            && std::fs::rename(&from, dir.join(name)).is_ok()
        {
            moved.push(name.to_string_lossy().into_owned());
        }
    }
    if moved.is_empty() { "no logs to archive".to_owned() } else { format!("archived {} to var/archive/{now}", moved.join(" and ")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_landings_make_a_pass_due_and_a_running_pass_holds_the_next() {
        let dir = tempfile::tempdir().unwrap();
        for _ in 0..9 {
            count_landing(dir.path());
        }
        assert!(!due(dir.path()));
        count_landing(dir.path());
        assert_eq!(landed_since(dir.path()), 10);
        assert!(due(dir.path()));
        std::fs::write(dir.path().join(RUNNING_FILE), std::process::id().to_string()).unwrap();
        assert!(!due(dir.path()), "a pass running holds the next");
    }

    #[test]
    fn lookback_backs_off_after_a_failed_pass() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("var")).unwrap();
        for _ in 0..EVERY {
            count_landing(dir.path());
        }
        std::fs::write(dir.path().join(FAILED_FILE), format!("{} {EVERY}", now())).unwrap();
        assert!(!due(dir.path()), "a failed pass is not retried at once");
        for _ in 0..EVERY {
            count_landing(dir.path());
        }
        assert!(due(dir.path()), "EVERY more cards end the backoff");
        std::fs::write(dir.path().join(COUNT_FILE), EVERY.to_string()).unwrap();
        std::fs::write(dir.path().join(FAILED_FILE), format!("{} {EVERY}", now() - BACKOFF_SECS)).unwrap();
        assert!(due(dir.path()), "an hour ends the backoff");
    }

    #[test]
    fn a_stale_marker_of_a_dead_pass_holds_nothing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("var")).unwrap();
        for _ in 0..EVERY {
            count_landing(dir.path());
        }
        // a pid no process holds: the largest a pid can be
        std::fs::write(dir.path().join(RUNNING_FILE), (u32::MAX - 1).to_string()).unwrap();
        assert!(!running(dir.path()));
        assert!(due(dir.path()));
    }

    #[test]
    fn the_archive_takes_the_logs_and_leaves_none_behind() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("var")).unwrap();
        std::fs::write(dir.path().join("var/retro.log"), "a\n").unwrap();
        let said = archive(dir.path());
        assert!(said.starts_with("archived retro.log to var/archive/"), "{said}");
        assert!(!dir.path().join("var/retro.log").exists());
    }
}
