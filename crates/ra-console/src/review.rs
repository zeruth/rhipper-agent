//! What the reviewer is told about where a card's change lives: the files the card names and
//! the run touched, and the `git diff --stat` of exactly those paths. A reviewer that read the
//! diff of `agent.rs` alone judged #159 unfinished because its timeout code lived in
//! `ra-tools/src/commands.rs`; the list and the stat say where to look before the diff does,
//! and on a shared tree the same path list is what tells the card's own change apart.

use std::fmt::Write as _;
use std::path::Path;

/// The heading of the dirty files of a shared tree that are not the card's change.
pub const NOT_THE_CARDS: &str = "not this card’s (dirty before the run)";

/// The `## Files of the card` section of the reviewer's brief: the card's `files:` first, the
/// ones the run's handoff added after them, one per line; then, under [`NOT_THE_CARDS`], the
/// files `stray` names: dirty in a shared tree but not this card's, which the reviewer is told
/// not to judge (#185 was sent back three times for the person's own uncommitted edits).
#[must_use]
pub fn files_section(card: &[String], changed: &[String], stray: &[String]) -> String {
    let mut text = String::from("\n## Files of the card\n");
    if card.is_empty() && changed.is_empty() {
        text.push_str("the card names no files and no run changed any\n");
    }
    for file in card.iter().filter(|f| !stray.contains(f)) {
        let _ = writeln!(text, "- {file}");
    }
    for file in changed.iter().filter(|f| !card.contains(f)) {
        let _ = writeln!(text, "- {file} (changed by the run, not on the card)");
    }
    if !stray.is_empty() {
        let _ = writeln!(text, "\n## {NOT_THE_CARDS}\nother edits in the shared tree; they are not in the diff below, do not judge them");
        for file in stray {
            let _ = writeln!(text, "- {file}");
        }
    }
    text
}

/// The `git diff --stat <base>` of `files` in `root`, as a section of the brief; the line that
/// says so when the stat is empty or git has nothing to say. `base` is `HEAD` in a project's
/// own tree and the merge base with the project's branch in a card's worktree, so the
/// checkpoints the walk commits on the card's branch stay in the reviewer's view; the catch-up
/// before the review moves that base to the project's HEAD, so the stat is the card's change
/// on the tree that lands, not what the project's branch brought in.
pub async fn diff_stat_section(root: &Path, base: &str, files: &[String]) -> String {
    let mut text = format!("\n## git diff --stat {base} of the card's files\n");
    let mut command = tokio::process::Command::new("git");
    command.args(["diff", "--stat", base, "--"]).args(files).current_dir(root);
    let stat = match command.output().await {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim_end().to_owned(),
        Ok(out) => format!("git diff --stat failed: {}", String::from_utf8_lossy(&out.stderr).trim()),
        Err(e) => format!("git diff --stat did not run: {e}"),
    };
    if stat.trim().is_empty() {
        let _ = writeln!(text, "no change against {base}");
    } else {
        let _ = writeln!(text, "{stat}");
    }
    text
}
