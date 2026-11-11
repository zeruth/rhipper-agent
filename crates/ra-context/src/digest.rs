//! A project's uncommitted leftovers for its digest: the files changed by cards that need
//! revision and are still dirty in the tree, so the next card knows the tree it inherits.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::Path;

use crate::TaskContext;

/// The heading of the leftovers' section in the digest.
pub const LEFTOVERS_HEADING: &str = "## Uncommitted leftovers";

/// The files that the cards `revising`, those whose state is needs-revision, changed in their
/// runs and that are still dirty in `workspace`: by path, with the cards that touched each.
/// A file committed or reverted since is not a leftover; a tree that is not a git repository,
/// or git missing, has none.
#[must_use]
pub fn leftovers(workspace: &Path, revising: &[i64]) -> BTreeMap<String, Vec<i64>> {
    let status = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(workspace)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
        .unwrap_or_default();
    let cards: Vec<(i64, Vec<String>)> = revising
        .iter()
        .map(|&id| (id, TaskContext::open(workspace, id).handoffs.iter().flat_map(|h| h.changed.iter().cloned()).collect()))
        .collect();
    leftovers_in(&status, &cards)
}

/// [`leftovers`] over a `git status --porcelain` listing and each card's changed paths.
#[must_use]
pub fn leftovers_in(status: &str, cards: &[(i64, Vec<String>)]) -> BTreeMap<String, Vec<i64>> {
    let dirty: std::collections::HashSet<String> =
        status.lines().filter_map(|l| l.get(3..)).map(|p| p.rsplit(" -> ").next().unwrap_or(p).trim_matches('"').to_owned()).collect();
    let mut found: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    for (id, paths) in cards {
        for path in paths.iter().filter(|p| dirty.contains(p.as_str())) {
            let ids = found.entry(path.clone()).or_default();
            if !ids.contains(id) {
                ids.push(*id);
            }
        }
    }
    found
}

/// The leftovers as a section of the digest, one file a line with its cards; empty when there
/// are none, so a clean tree adds nothing.
#[must_use]
pub fn section(leftovers: &BTreeMap<String, Vec<i64>>) -> String {
    if leftovers.is_empty() {
        return String::new();
    }
    let mut text = format!("{LEFTOVERS_HEADING}\n");
    for (path, ids) in leftovers {
        let cards: Vec<String> = ids.iter().map(|id| format!("#{id}")).collect();
        let _ = writeln!(text, "- {path} ({})", cards.join(" "));
    }
    text
}
