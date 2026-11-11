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

/// Whether `id` is `ancestor` or sits somewhere under it: the check a move makes before it
/// puts a node under one of its own descendants. A missing id is under nothing.
#[must_use]
pub fn is_within(nodes: &[Node], id: i64, ancestor: i64) -> bool {
    let by_id: HashMap<i64, &Node> = nodes.iter().map(|n| (n.id, n)).collect();
    let mut cursor = by_id.get(&id).map(|n| n.id);
    while let Some(current) = cursor {
        if current == ancestor {
            return true;
        }
        cursor = by_id.get(&current).and_then(|n| n.parent);
    }
    false
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

/// The files a card works in, and where they come from: its own when it names any, else those of
/// the nearest ancestor that does, with that ancestor. A section or milestone carries `files:` once
/// for the cards under it. Empty, from no one, when nothing up the chain names a file.
#[must_use]
pub fn files_of<'a>(nodes: &'a [Node], node: &'a Node) -> (&'a [String], Option<&'a Node>) {
    let by_id: HashMap<i64, &Node> = nodes.iter().map(|n| (n.id, n)).collect();
    files_in(&by_id, node)
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

/// The faults `rc check` reports, one line each with the id, in tree order: placeholder text in
/// a title, objective or acceptance; a task or subtask without acceptance or without files, its
/// own or an ancestor's; a waits-on that names no node; a waits-on cycle; a complexity outside
/// 1..=5 on a card. Accepted
/// and cancelled cards are past fixing and are only checked for the structural faults, the
/// dangling and cyclic waits-on. Empty when the roadmap is clean.
#[must_use]
pub fn lint(nodes: &[Node]) -> Vec<String> {
    let by_id: HashMap<i64, &Node> = nodes.iter().map(|n| (n.id, n)).collect();
    let cyclic = cyclic_ids(nodes, &by_id);
    let mut out = Vec::new();
    for node in nodes {
        let id = node.id;
        let card = matches!(node.kind, NodeKind::Task | NodeKind::Subtask);
        let done = matches!(node.state, TaskState::Accepted | TaskState::Cancelled);
        if !done {
            for (field, text) in
                [("title", Some(node.title.as_str())), ("objective", node.objective.as_deref()), ("acceptance", node.acceptance.as_deref())]
            {
                if text.is_some_and(is_placeholder) {
                    out.push(format!("#{id}: the {field} is placeholder text"));
                }
            }
            if card && node.acceptance.as_deref().is_none_or(|a| a.trim().is_empty()) {
                out.push(format!("#{id}: no acceptance"));
            }
            if card && files_in(&by_id, node).0.is_empty() {
                out.push(format!("#{id}: no files"));
            }
            if card && !(1..=5).contains(&node.complexity) {
                out.push(format!("#{id}: complexity {} is outside 1..5", node.complexity));
            }
        }
        for dep in &node.depends_on {
            if !by_id.contains_key(dep) {
                out.push(format!("#{id}: waits on #{dep}, which is not on the roadmap"));
            }
        }
        if cyclic.contains(&id) {
            out.push(format!("#{id}: its waits-on come back to it"));
        }
    }
    out
}

/// Whether a field is template text rather than words about the work: empty, a bracketed hint
/// such as `<the objective>`, or a bare `tbd`, `todo` or ellipsis.
fn is_placeholder(text: &str) -> bool {
    let text = text.trim();
    text.is_empty()
        || (text.starts_with('<') && text.ends_with('>'))
        || matches!(text.to_ascii_lowercase().trim_end_matches(['.', ':']), "tbd" | "todo" | "tba" | "…" | "")
}

/// The ids on a waits-on cycle: a depth-first walk that meets a node still on its own path.
fn cyclic_ids(nodes: &[Node], by_id: &HashMap<i64, &Node>) -> Vec<i64> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Open,
        Done,
    }
    let mut marks: HashMap<i64, Mark> = HashMap::new();
    let mut cyclic = Vec::new();
    for start in nodes {
        if marks.contains_key(&start.id) {
            continue;
        }
        // an explicit stack of (node, next dependency index), so a deep chain never overflows
        let mut path: Vec<(i64, usize)> = vec![(start.id, 0)];
        marks.insert(start.id, Mark::Open);
        while let Some(&mut (id, ref mut next)) = path.last_mut() {
            let deps = by_id.get(&id).map(|n| n.depends_on.as_slice()).unwrap_or_default();
            let Some(&dep) = deps.get(*next) else {
                marks.insert(id, Mark::Done);
                path.pop();
                continue;
            };
            *next += 1;
            match marks.get(&dep) {
                Some(Mark::Open) => {
                    // every node from the repeated one to the top of the path is on the cycle
                    let from = path.iter().position(|(on, _)| *on == dep).unwrap_or(0);
                    for (on, _) in &path[from..] {
                        if !cyclic.contains(on) {
                            cyclic.push(*on);
                        }
                    }
                }
                None if by_id.contains_key(&dep) => {
                    marks.insert(dep, Mark::Open);
                    path.push((dep, 0));
                }
                // a finished node leads to no new cycle; a dangling waits-on is reported on its
                // own and leads nowhere
                Some(Mark::Done) | None => {}
            }
        }
    }
    cyclic
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

fn files_in<'a>(by_id: &HashMap<i64, &'a Node>, node: &'a Node) -> (&'a [String], Option<&'a Node>) {
    if !node.files.is_empty() {
        return (&node.files, None);
    }
    let mut parent = node.parent.and_then(|id| by_id.get(&id).copied());
    while let Some(ancestor) = parent {
        if !ancestor.files.is_empty() {
            return (&ancestor.files, Some(ancestor));
        }
        parent = ancestor.parent.and_then(|id| by_id.get(&id).copied());
    }
    (&[], None)
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
