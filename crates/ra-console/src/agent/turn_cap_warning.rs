//! The warning a worker gets five turns before the turn cap: one message, so the run ends with
//! its report instead of `unfinished (no report)`.

use std::path::Path;

/// The environment variable a run sets to its turn cap, for the post hook to count against.
pub(super) const CAP_VAR: &str = "RHIPPER_TURN_CAP";
/// The tool calls the post hook has counted in this run; a run starts it afresh.
pub(super) const TURNS_FILE: &str = "var/runs/turns";
/// How many turns before the cap the worker is told to write its report.
const WARN_BEFORE: u32 = 5;

/// What the post hook adds after a tool call: the warning when this call is the one
/// [`WARN_BEFORE`] turns from the run's cap, nothing for any other call or a run with no cap.
#[must_use]
pub fn turn_cap_warning(root: &Path) -> Option<String> {
    let cap = std::env::var(CAP_VAR).ok().and_then(|v| v.trim().parse().ok())?;
    warning_at(root, cap)
}

/// [`turn_cap_warning`] for a known cap, counting this call into the run's turn file.
fn warning_at(root: &Path, cap: u32) -> Option<String> {
    let memo = root.join(TURNS_FILE);
    let turn = std::fs::read_to_string(&memo).ok().and_then(|t| t.trim().parse::<u32>().ok()).unwrap_or(0) + 1;
    if let Some(dir) = memo.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&memo, turn.to_string());
    (cap > WARN_BEFORE && turn == cap - WARN_BEFORE).then(|| {
        format!(
            "Turn {turn} of {cap}: the run ends in {WARN_BEFORE} turns. Stop working now and write your report: what is done and what is left."
        )
    })
}

#[cfg(test)]
mod tests;
