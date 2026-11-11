//! The tree walks over a list of nodes: the subtree of a node, and the depth of each. The list
//! comes from the store in tree order, a parent before its children, and the walks keep that order.

use std::collections::HashMap;

use crate::model::Node;

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
