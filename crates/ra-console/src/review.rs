//! What the reviewer is told about where a card's change lives: the files the card names and
//! its runs touched, and the `git diff --stat` of exactly those paths. A reviewer that read the
//! diff of `agent.rs` alone judged #159 unfinished because its timeout code lived in
//! `ra-tools/src/commands.rs`; the list and the stat say where to look before the diff does,
//! and on a shared tree the same path list is what tells the card's own change apart.
//!
//! The scope is the card's whole work, not the last run's: on a card's branch every file the
//! branch changed since its base and every untracked one, so a checkpoint an earlier run left
//! is not judged missing (runelite #44 spent four rounds on files that had landed). The head
//! of a run's handoff, what the tree is at and what happened to it since the last run, is read
//! here too ([`since_last`], [`tree_of`]).

use std::fmt::Write as _;
use std::path::Path;

#[cfg(test)]
mod tests;

/// The heading of the dirty files of a shared tree that are not the card's change.
pub const NOT_THE_CARDS: &str = "not this card’s (dirty before the run)";

/// The `## Files of the card` section of the reviewer's brief: the card's `files:` first, the
/// ones its runs changed after them, one per line; then, under [`NOT_THE_CARDS`], the files
/// `stray` names: dirty in a shared tree but not this card's, which the reviewer is told not
/// to judge (#185 was sent back three times for the person's own uncommitted edits).
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
        let _ = writeln!(text, "- {file} (changed by the card's runs, not on the card)");
    }
    if !stray.is_empty() {
        let _ = writeln!(text, "\n## {NOT_THE_CARDS}\nother edits in the shared tree; they are not in the diff below, do not judge them");
        for file in stray {
            let _ = writeln!(text, "- {file}");
        }
    }
    text
}

/// The paths a review of a card reads: `named` (the card's files and every handoff's changed
/// ones) first, then, when the run acted on the card's own `branch`, every path the branch
/// changed since `base` (its commits and its working tree) and every untracked one. On the
/// project's shared tree the named paths alone, which tell the card's change from the others'.
pub async fn scope(root: &Path, base: &str, branch: bool, named: &[String]) -> Vec<String> {
    let mut files: Vec<String> = Vec::new();
    let mut add = |file: &str| {
        if !file.is_empty() && !files.iter().any(|f| f == file) {
            files.push(file.to_owned());
        }
    };
    for file in named {
        add(file);
    }
    if branch {
        for args in [&["diff", "--name-only", base][..], &["ls-files", "--others", "--exclude-standard"][..]] {
            for file in git(root, args).await.unwrap_or_default().lines() {
                add(file.trim());
            }
        }
    }
    files
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

/// What happened to the tree at `root` since `tip`, where the last run of the card left it:
/// commits landed on it, a merge, or a reset, as [`ra_context::since_line`] words it; `None`
/// when the tree is where the run left it or git cannot say.
pub async fn since_last(root: &Path, tip: &str) -> Option<String> {
    let now = git(root, &["rev-parse", "--short=12", "HEAD"]).await?;
    let ancestor = tokio::process::Command::new("git")
        .args(["merge-base", "--is-ancestor", tip, "HEAD"])
        .current_dir(root)
        .output()
        .await
        .is_ok_and(|out| out.status.success());
    let log = if ancestor {
        git(root, &["log", "--format=%h %p%x09%s", &format!("{tip}..HEAD")]).await.unwrap_or_default()
    } else {
        String::new()
    };
    ra_context::since_line(tip, &now, ancestor, &log)
}

/// The tree at `root` as a run ends, for the head of its handoff: its tip, and on the card's
/// `branch` the commits since `base`; `since` is what [`since_last`] said when the run began.
pub async fn tree_of(root: &Path, base: &str, branch: Option<&str>, since: Option<String>) -> ra_context::Tree {
    let tip = git(root, &["rev-parse", "--short=12", "HEAD"]).await.unwrap_or_default();
    let commits = match branch {
        Some(_) if base != "HEAD" => {
            git(root, &["log", "--format=%h %s", &format!("{base}..HEAD")]).await.unwrap_or_default().lines().map(str::to_owned).collect()
        }
        _ => Vec::new(),
    };
    ra_context::Tree { tip, branch: branch.map(str::to_owned), commits, since }
}

/// One git command in `dir`, its stdout trimmed; `None` when it does not run or fails.
async fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = tokio::process::Command::new("git").args(args).current_dir(dir).output().await.ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}
