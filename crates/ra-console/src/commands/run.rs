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

/// The next milestone the planner may fill: the first in the file's order, among the open ones,
/// with no task or subtask anywhere under it. A milestone that holds cards already is the
/// person's to plan further, so it is passed over.
#[must_use]
pub fn next_unplanned(nodes: &[Node]) -> Option<&Node> {
    nodes.iter().find(|n| {
        n.kind == NodeKind::Milestone
            && !matches!(n.state, TaskState::Accepted | TaskState::Cancelled)
            && !tree::subtree(nodes, n.id).iter().any(|c| matches!(c.kind, NodeKind::Task | NodeKind::Subtask))
    })
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
    let plan = plan(store, milestone.id, worker).await?;
    let mut lines = plan.add(store).await?;
    let notice =
        format!("planned #{} {}: {} added as defined by {}", plan.id, plan.title, plan.outline.summary(), crate::agent::short(&plan.model));
    crate::pair::event(root, &notice);
    lines.push(notice);
    Ok(Some(lines))
}
