//! The link to a paired session: an external agent (a Claude Code session, say) that drives
//! and reviews this agent. The agent cannot push into such a session, but the session can
//! watch a file; so the agent keeps `var/pair.log`, one line per event — a run's start, its
//! end with the outcome, a walk's end, a note — and the pairing skill arms a watch on it first
//! thing. `rc pair` prints the latest.

use std::path::Path;

/// The feed, under the workspace: `time<TAB>text`.
pub const PAIR_FILE: &str = "var/pair.log";

/// Appends one event. A feed that cannot be written is nobody's failure: the run goes on.
pub fn event(root: &Path, text: &str) {
    use std::io::Write;
    let path = root.join(PAIR_FILE);
    let _ = std::fs::create_dir_all(root.join("var"));
    let when = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{when}\t{}", text.replace(['\t', '\n'], " "));
    }
}

/// The latest `limit` events, oldest first; none without a feed.
pub fn recent(root: &Path, limit: usize) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(root.join(PAIR_FILE)) else { return Vec::new() };
    let lines: Vec<String> = text.lines().map(|l| l.split_once('\t').map_or(l, |(_, t)| t).to_owned()).collect();
    lines[lines.len().saturating_sub(limit)..].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_feed_keeps_its_events_in_order_and_one_line_each() {
        let dir = tempfile::tempdir().unwrap();
        assert!(recent(dir.path(), 5).is_empty());
        event(dir.path(), "start #7 A card · claude-fable-5-1");
        event(dir.path(), "end #7 ✓ implemented\nwith a second line");
        event(dir.path(), "note the tooltip is slow");
        assert_eq!(recent(dir.path(), 2), ["end #7 ✓ implemented with a second line", "note the tooltip is slow"]);
        assert_eq!(recent(dir.path(), 10).len(), 3);
    }
}
