//! Where a planned card goes: a project's own friction to its standing Look-back milestone, the
//! agent's to the agent's. The agent is the person's local fork of Rhipper Agent, loaded as a
//! project of its own: what a hook refused, a check misread or a walk got wrong in any project
//! is fixed once in its source and every project gains, so a look-back anywhere edits the
//! agent's roadmap. Before an agent card lands, the agent's open cards are read, and a report
//! one of them already covers joins it as a note instead of making a second card.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use ra_roadmap::model::{Node, NodeKind, TaskState};
use ra_roadmap::store::Store;

/// The title of the standing milestone a look-back's cards go under, in every project.
pub const MILESTONE: &str = "Look-back";

/// What the milestone is for, written when a pass makes it.
const OBJECTIVE: &str =
    "The cards the look-back passes plan from the evidence of the runs: the friction a walk met, fixed where it was met.";

/// Whose problem a planned card is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Owner {
    /// The project's own code.
    #[default]
    Project,
    /// The agent's: its hooks, gotchas, checks, review, landing, prompts or `rc` commands.
    Agent,
}

impl Owner {
    /// The owner an `owner:` line names; anything but `agent` is the project's.
    #[must_use]
    pub fn of(word: &str) -> Self {
        if word.trim().eq_ignore_ascii_case("agent") { Self::Agent } else { Self::Project }
    }
}

/// Whether `root` is the agent itself: a project holding the agent's own source.
#[must_use]
pub fn is_agent(root: &Path) -> bool {
    root.join("crates").join("ra-console").join("Cargo.toml").is_file() && root.join("rhipper.md").is_file()
}

/// The agent's project, when it is loaded: `root` itself when it is the agent, else the first
/// project the window has open that is. `None` keeps an agent card at home.
#[must_use]
pub fn agent_home(root: &Path) -> Option<PathBuf> {
    if is_agent(root) {
        return Some(root.to_path_buf());
    }
    crate::hub::recent().into_iter().find(|project| is_agent(project))
}

/// The standing Look-back milestone of a roadmap, when it has one.
#[must_use]
pub fn milestone(nodes: &[Node]) -> Option<&Node> {
    nodes.iter().find(|n| n.kind == NodeKind::Milestone && n.title.eq_ignore_ascii_case(MILESTONE))
}

/// The id of the store's Look-back milestone, made under its project on the first pass.
///
/// # Errors
/// When the roadmap cannot be read or written, or it has no project to hold the milestone.
pub async fn ensure_milestone(store: &Store) -> anyhow::Result<i64> {
    let nodes = store.all().await?;
    if let Some(found) = milestone(&nodes) {
        return Ok(found.id);
    }
    let project = nodes.iter().find(|n| n.kind == NodeKind::Project).map(|n| n.id);
    let project = project.ok_or_else(|| anyhow::anyhow!("no project on the roadmap to hold the {MILESTONE} milestone"))?;
    store.insert(Some(project), NodeKind::Milestone, MILESTONE, Some(OBJECTIVE), Some("Every child accepted."), 3).await
}

/// The words of a title that say what it is about: lowercase, four letters or more, none of the
/// words every card uses.
fn words(title: &str) -> BTreeSet<String> {
    const COMMON: [&str; 12] = ["that", "this", "with", "from", "when", "into", "card", "cards", "runs", "make", "makes", "every"];
    title
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
        .map(str::to_ascii_lowercase)
        .filter(|w| w.len() >= 4 && !COMMON.contains(&w.as_str()))
        .collect()
}

/// Whether two titles name the same change: most of the shorter one's words are in the other.
#[must_use]
pub fn same_change(a: &str, b: &str) -> bool {
    let (a, b) = (words(a), words(b));
    let shorter = a.len().min(b.len());
    shorter >= 3 && a.intersection(&b).count() * 5 >= shorter * 3
}

/// The agent's open cards a planned title duplicates: the first that names the same change.
#[must_use]
pub fn duplicate<'a>(nodes: &'a [Node], title: &str) -> Option<&'a Node> {
    nodes
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Task | NodeKind::Subtask))
        .filter(|n| !matches!(n.state, TaskState::Accepted | TaskState::Cancelled))
        .find(|n| same_change(&n.title, title))
}

/// The titles of the agent's open cards, for the planner to leave alone: the Look-back
/// milestone's first, then the rest, at most `limit`.
#[must_use]
pub fn open_titles(nodes: &[Node], limit: usize) -> Vec<String> {
    let under = milestone(nodes).map(|m| m.id);
    let open =
        |n: &&Node| matches!(n.kind, NodeKind::Task | NodeKind::Subtask) && !matches!(n.state, TaskState::Accepted | TaskState::Cancelled);
    let (mut first, rest): (Vec<&Node>, Vec<&Node>) = nodes.iter().filter(open).partition(|n| under.is_some() && n.parent == under);
    first.extend(rest);
    first.into_iter().take(limit).map(|n| format!("#{} {}", n.id, n.title)).collect()
}

/// The name a project goes by in another's roadmap: its directory's.
#[must_use]
pub fn name_of(root: &Path) -> String {
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    root.file_name().map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_reports_of_one_refused_command_are_one_change_and_unrelated_titles_are_not() {
        assert!(same_change("Allow git merge-base in the read-only list", "Let workers run git merge-base, read-only"));
        assert!(!same_change("Allow git merge-base in the read-only list", "Quote the clippy warnings in the rerun prompt"));
        // too few words to tell
        assert!(!same_change("Fix lint", "Fix lint"));
        assert_eq!(Owner::of(" Agent "), Owner::Agent);
        assert_eq!(Owner::of("runelite"), Owner::Project);
    }
}
