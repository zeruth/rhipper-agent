//! The tests of the tree walks.

use super::*;
use crate::model::{NodeKind, TaskState};

fn node(id: i64, parent: Option<i64>) -> Node {
    Node {
        id,
        parent,
        kind: NodeKind::Task,
        title: String::new(),
        objective: None,
        acceptance: None,
        files: Vec::new(),
        session: None,
        depends_on: Vec::new(),
        state: TaskState::Defined,
        assignee: None,
        complexity: 3,
        notes: None,
        assessed: None,
    }
}

fn ids(nodes: &[&Node]) -> Vec<i64> {
    nodes.iter().map(|n| n.id).collect()
}

#[test]
fn the_subtree_walks_preorder() {
    let nodes = vec![node(1, None), node(2, Some(1)), node(3, Some(2)), node(4, Some(2)), node(5, Some(1))];
    assert_eq!(ids(&subtree(&nodes, 2)), vec![2, 3, 4]);
    assert_eq!(ids(&subtree(&nodes, 1)), vec![1, 2, 3, 4, 5]);
    assert_eq!(ids(&subtree(&nodes, 5)), vec![5]);
    assert!(subtree(&nodes, 9).is_empty());
}

#[test]
fn the_subtree_keeps_the_list_order_of_siblings() {
    // a later child with a lower id still follows its earlier sibling
    let nodes = vec![node(1, None), node(3, Some(1)), node(2, Some(1))];
    assert_eq!(ids(&subtree(&nodes, 1)), vec![1, 3, 2]);
}

#[test]
fn depths_grow_down_the_tree() {
    let nodes = vec![node(1, None), node(2, Some(1)), node(3, Some(2))];
    assert_eq!(depth(&nodes, 1), 0);
    assert_eq!(depth(&nodes, 2), 1);
    assert_eq!(depth(&nodes, 3), 2);
    assert_eq!(depth(&nodes, 9), 0);
    let all = depths(&nodes);
    assert_eq!(all[&3], 2);
    assert_eq!(all.len(), 3);
}
