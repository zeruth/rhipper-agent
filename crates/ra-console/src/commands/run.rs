//! What a walk does when no card is ready: with an active `plan-next` gambit it asks the planner
//! for the next milestone that has no cards yet, adds the cards as defined and tells the pair
//! feed in one line; without the gambit the walk ends as it always did.

use std::path::Path;

use ra_roadmap::model::{Gambit, GambitState, Node, NodeKind, TaskState};
use ra_roadmap::store::Store;
use ra_roadmap::tree;

use crate::agent::{Worker, plan};

/// Whether an active gambit lets the walk plan the next milestone.
#[must_use]
pub fn plans_next(gambits: &[Gambit]) -> bool {
    gambits.iter().any(|g| g.state == GambitState::Active && g.plan_next)
}

/// An open milestone with no task or subtask anywhere under it.
fn is_unplanned(nodes: &[Node], n: &Node) -> bool {
    n.kind == NodeKind::Milestone
        && !matches!(n.state, TaskState::Accepted | TaskState::Cancelled)
        && !tree::subtree(nodes, n.id).iter().any(|c| matches!(c.kind, NodeKind::Task | NodeKind::Subtask))
}

/// The node an unplanned milestone waits on, its own wait or one above it, that is not accepted yet.
fn open_wait(nodes: &[Node], n: &Node) -> Option<i64> {
    let holder = tree::holder_of(nodes, n)?;
    holder.depends_on.iter().copied().find(|dep| nodes.iter().find(|d| d.id == *dep).is_none_or(|d| d.state != TaskState::Accepted))
}

/// The next milestone the planner may fill: the first in the file's order, among the open ones,
/// with no task or subtask anywhere under it and no wait still open. A milestone that holds cards
/// already is the person's to plan further, and one that waits on unfinished work would cite code
/// that does not exist yet, so both are passed over.
#[must_use]
pub fn next_unplanned(nodes: &[Node]) -> Option<&Node> {
    nodes.iter().find(|n| is_unplanned(nodes, n) && open_wait(nodes, n).is_none())
}

/// The milestones passed over for an open wait, as notice lines, in the file's order up to the one
/// planned; each pair is said once per process, not on every walk step.
pub(super) fn passed_over(nodes: &[Node], planned: i64) -> Vec<String> {
    static SAID: std::sync::Mutex<Vec<(i64, i64)>> = std::sync::Mutex::new(Vec::new());
    let mut said = SAID.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut lines = Vec::new();
    for n in nodes.iter().take_while(|n| n.id != planned).filter(|n| is_unplanned(nodes, n)) {
        if let Some(dep) = open_wait(nodes, n)
            && !said.contains(&(n.id, dep))
        {
            said.push((n.id, dep));
            lines.push(format!("passed over #{} {}: it waits on #{dep}, which is not accepted", n.id, n.title));
        }
    }
    lines
}

/// Plans the milestone and adds its cards as defined; the lines of the plan, then the notice
/// that went to the pair feed. `None` when no milestone is left to plan, which the walk's end
/// line says.
///
/// # Errors
/// When the planner fails or the store cannot take the cards.
pub async fn plan_next(store: &Store, root: &Path, worker: &Worker) -> anyhow::Result<Option<Vec<String>>> {
    let nodes = store.all().await?;
    let Some(milestone) = next_unplanned(&nodes) else { return Ok(None) };
    let skipped = passed_over(&nodes, milestone.id);
    for line in &skipped {
        crate::pair::event(root, line);
    }
    let plan = plan(store, milestone.id, worker).await?;
    let mut lines = skipped;
    lines.extend(plan.add(store).await?);
    let notice =
        format!("planned #{} {}: {} added as defined by {}", plan.id, plan.title, plan.outline.summary(), crate::agent::short(&plan.model));
    crate::pair::event(root, &notice);
    lines.push(notice);
    Ok(Some(lines))
}
