//! What the reviewer is told about where a card's change lives: the files the card names and
//! the run touched, and the `git diff --stat` of exactly those paths. A reviewer that read the
//! diff of `agent.rs` alone judged #159 unfinished because its timeout code lived in
//! `ra-tools/src/commands.rs`; the list and the stat say where to look before the diff does,
//! and on a shared tree the same path list is what tells the card's own change apart.

use std::fmt::Write as _;
use std::path::Path;

/// The `## Files of the card` section of the reviewer's brief: the card's `files:` first, the
/// ones the run's handoff added after them, one per line.
#[must_use]
pub fn files_section(card: &[String], changed: &[String]) -> String {
    let mut text = String::from("\n## Files of the card\n");
    if card.is_empty() && changed.is_empty() {
        text.push_str("the card names no files and no run changed any\n");
        return text;
    }
    for file in card {
        let _ = writeln!(text, "- {file}");
    }
    for file in changed.iter().filter(|f| !card.contains(f)) {
        let _ = writeln!(text, "- {file} (changed by the run, not on the card)");
    }
    text
}

/// The `git diff --stat HEAD` of `files` in `root`, as a section of the brief; the line that
/// says so when the stat is empty or git has nothing to say.
pub async fn diff_stat_section(root: &Path, files: &[String]) -> String {
    let mut text = String::from("\n## git diff --stat HEAD of the card's files\n");
    let mut command = tokio::process::Command::new("git");
    command.args(["diff", "--stat", "HEAD", "--"]).args(files).current_dir(root);
    let stat = match command.output().await {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim_end().to_owned(),
        Ok(out) => format!("git diff --stat failed: {}", String::from_utf8_lossy(&out.stderr).trim()),
        Err(e) => format!("git diff --stat did not run: {e}"),
    };
    if stat.trim().is_empty() {
        text.push_str("no change against HEAD\n");
    } else {
        let _ = writeln!(text, "{stat}");
    }
    text
}
