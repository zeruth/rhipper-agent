//! The tree walks over a list of nodes: the subtree of a node, the depth of each, and the state a
//! card stands in once its waits-on are read. The list comes from the store in tree order, a parent
//! before its children, and the walks keep that order.

use std::collections::HashMap;

use crate::model::{Gambit, GambitState, Node, NodeKind, TaskState};

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

/// The ready card the walk takes first: the best of [`ranked`], the one `rc next` names.
#[must_use]
pub fn next_ready<'a>(nodes: &'a [Node], gambits: &'a [Gambit]) -> Option<&'a Node> {
    ranked(nodes, gambits).into_iter().next().map(|s| s.node)
}

/// What places a card in the walk: its priority, then the gambits that pull it forward.
#[derive(Debug, Clone, PartialEq)]
pub struct Score<'a> {
    pub node: &'a Node,
    /// The card's priority; 0 when the person set none.
    pub priority: u8,
    /// The summed weights of `gambits`.
    pub weight: u32,
    /// The active gambits whose focus matches the card, in the order of the file.
    pub gambits: Vec<&'a Gambit>,
}

impl Score<'_> {
    /// The line `rc next --why` prints: the card, its priority, the summed weight and the
    /// gambits that moved it, `#3 A local route · p5 · +6 · g1 Local first {w3}, g2 …`.
    #[must_use]
    pub fn line(&self) -> String {
        let priority = self.node.priority.map_or_else(|| "p-".to_owned(), |p| format!("p{p}"));
        let moved = if self.gambits.is_empty() {
            "no gambit".to_owned()
        } else {
            self.gambits.iter().map(|g| format!("g{} {} {{w{}}}", g.id, g.text, g.weight)).collect::<Vec<_>>().join(", ")
        };
        format!("#{} {} · {priority} · +{} · {moved}", self.node.id, self.node.title, self.weight)
    }
}

/// The score of one card: its priority, and every active gambit whose focus matches it. A focus
/// word matches when the card's title, its milestone's title or one of its files (its own or the
/// nearest ancestor's) contains it, case apart; a gambit with no focus matches every card.
#[must_use]
pub fn score<'a>(nodes: &'a [Node], node: &'a Node, gambits: &'a [Gambit]) -> Score<'a> {
    let by_id: HashMap<i64, &Node> = nodes.iter().map(|n| (n.id, n)).collect();
    score_in(&by_id, node, gambits)
}

/// The ready cards, the walk's pick first: by priority, then by the summed weight of the matching
/// gambits, then in tree order. The sort is stable, so the tiebreak is the order of the file.
#[must_use]
pub fn ranked<'a>(nodes: &'a [Node], gambits: &'a [Gambit]) -> Vec<Score<'a>> {
    let by_id: HashMap<i64, &Node> = nodes.iter().map(|n| (n.id, n)).collect();
    let ready = nodes.iter().filter(|n| effective_in(&by_id, n) == TaskState::Ready);
    rank(nodes, ready, gambits)
}

/// `candidates` scored and ordered as [`ranked`] orders the ready cards; for a caller with its
/// own idea of readiness, such as a batch that counts implemented waits-on as met.
#[must_use]
pub fn rank<'a>(nodes: &'a [Node], candidates: impl IntoIterator<Item = &'a Node>, gambits: &'a [Gambit]) -> Vec<Score<'a>> {
    let by_id: HashMap<i64, &Node> = nodes.iter().map(|n| (n.id, n)).collect();
    let mut scores: Vec<Score<'a>> = candidates.into_iter().map(|n| score_in(&by_id, n, gambits)).collect();
    scores.sort_by_key(|s| std::cmp::Reverse((s.priority, s.weight)));
    scores
}

fn score_in<'a>(by_id: &HashMap<i64, &'a Node>, node: &'a Node, gambits: &'a [Gambit]) -> Score<'a> {
    let title = node.title.to_lowercase();
    let milestone = milestone_in(by_id, node).map(|m| m.title.to_lowercase());
    let files: Vec<String> = files_in(by_id, node).0.iter().map(|f| f.to_lowercase()).collect();
    let matched: Vec<&Gambit> = gambits
        .iter()
        .filter(|g| g.state == GambitState::Active)
        .filter(|g| {
            g.focus.iter().any(|word| {
                let word = word.to_lowercase();
                title.contains(&word) || milestone.as_deref().is_some_and(|m| m.contains(&word)) || files.iter().any(|f| f.contains(&word))
            }) || g.focus.is_empty()
        })
        .collect();
    let weight = matched.iter().map(|g| u32::from(g.weight)).sum();
    Score { node, priority: node.priority.unwrap_or(0), weight, gambits: matched }
}

/// The nearest milestone above the node, itself included.
fn milestone_in<'a>(by_id: &HashMap<i64, &'a Node>, node: &'a Node) -> Option<&'a Node> {
    let mut cursor = Some(node);
    while let Some(current) = cursor {
        if current.kind == NodeKind::Milestone {
            return Some(current);
        }
        cursor = current.parent.and_then(|id| by_id.get(&id).copied());
    }
    None
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
