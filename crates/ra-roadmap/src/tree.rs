//! The tree walks over a list of nodes: the subtree of a node, the depth of each, and the state a
//! card stands in once its waits-on are read. The list comes from the store in tree order, a parent
//! before its children, and the walks keep that order.

use std::collections::HashMap;

use crate::model::{Node, NodeKind, TaskState};

/// The subtree of `root` in preorder: the node, then each child's subtree, in the order of the
/// list. Empty when no node has that id.
pub fn subtree(nodes: &[Node], root: i64) -> Vec<&Node> {
    let mut kids: HashMap<i64, Vec<&Node>> = HashMap::new();
    for node in nodes {
        if let Some(parent) = node.parent {
            kids.entry(parent).or_default().push(node);
        }
    }
    let Some(root) = nodes.iter().find(|n| n.id == root) else { return Vec::new() };
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        out.push(node);
        if let Some(children) = kids.get(&node.id) {
            stack.extend(children.iter().rev());
        }
    }
    out
}

/// The depth of the node with `id`: the distance from its project root. A missing id is depth 0.
pub fn depth(nodes: &[Node], id: i64) -> usize {
    let by_id: HashMap<i64, &Node> = nodes.iter().map(|n| (n.id, n)).collect();
    depth_in(&by_id, id)
}

/// The depth of every node, by id.
pub fn depths(nodes: &[Node]) -> HashMap<i64, usize> {
    let by_id: HashMap<i64, &Node> = nodes.iter().map(|n| (n.id, n)).collect();
    nodes.iter().map(|n| (n.id, depth_in(&by_id, n.id))).collect()
}

/// The state a card stands in once its waits-on are read, computed and never written: a defined
/// task or subtask is ready when every node it waits on is accepted (a card waiting on nothing is
/// ready), blocked when one of them failed, was cancelled or is gone, and stays defined while one
/// is still open. Any other state, and any node above a task, is the stored one.
#[must_use]
pub fn effective_state(nodes: &[Node], node: &Node) -> TaskState {
    let by_id: HashMap<i64, &Node> = nodes.iter().map(|n| (n.id, n)).collect();
    effective_in(&by_id, node)
}

/// The effective state of every node, by id.
#[must_use]
pub fn effective_states(nodes: &[Node]) -> HashMap<i64, TaskState> {
    let by_id: HashMap<i64, &Node> = nodes.iter().map(|n| (n.id, n)).collect();
    nodes.iter().map(|n| (n.id, effective_in(&by_id, n))).collect()
}

/// The first card in tree order whose effective state is ready: the one `rc next` names.
#[must_use]
pub fn next_ready(nodes: &[Node]) -> Option<&Node> {
    let by_id: HashMap<i64, &Node> = nodes.iter().map(|n| (n.id, n)).collect();
    nodes.iter().find(|n| effective_in(&by_id, n) == TaskState::Ready)
}

fn effective_in(by_id: &HashMap<i64, &Node>, node: &Node) -> TaskState {
    if node.state != TaskState::Defined || !matches!(node.kind, NodeKind::Task | NodeKind::Subtask) {
        return node.state;
    }
    let mut ready = true;
    for dep in &node.depends_on {
        match by_id.get(dep).map(|n| n.state) {
            Some(TaskState::Accepted) => {}
            // a dependency that will never be accepted, or that left the roadmap, holds the card
            Some(TaskState::Failed | TaskState::Cancelled) | None => return TaskState::Blocked,
            Some(_) => ready = false,
        }
    }
    if ready { TaskState::Ready } else { TaskState::Defined }
}

fn depth_in(by_id: &HashMap<i64, &Node>, id: i64) -> usize {
    let mut level = 0;
    let mut parent = by_id.get(&id).and_then(|n| n.parent);
    while let Some(id) = parent {
        parent = by_id.get(&id).and_then(|n| n.parent);
        level += 1;
    }
    level
}

#[cfg(test)]
mod tests;
