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
        session_at: None,
        session_of: None,
        depends_on: Vec::new(),
        state: TaskState::Defined,
        assignee: None,
        complexity: 3,
        effort: crate::model::Effort::default(),
        estimate: None,
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

fn task(id: i64, state: TaskState, depends_on: &[i64]) -> Node {
    Node { state, depends_on: depends_on.to_vec(), ..node(id, Some(1)) }
}

#[test]
fn ready_blocked_and_plain_defined_are_told_apart() {
    // #2 accepted, #3 failed, #4 still in progress; #5 waits on the accepted one, #6 on the failed
    // one, #7 on the open one, #8 on nothing, #9 on a card that left the roadmap
    let mut root = node(1, None);
    root.kind = NodeKind::Section;
    let nodes = vec![
        root,
        task(2, TaskState::Accepted, &[]),
        task(3, TaskState::Failed, &[]),
        task(4, TaskState::InProgress, &[]),
        task(5, TaskState::Defined, &[2]),
        task(6, TaskState::Defined, &[3]),
        task(7, TaskState::Defined, &[2, 4]),
        task(8, TaskState::Defined, &[]),
        task(9, TaskState::Defined, &[99]),
    ];
    let states = effective_states(&nodes);
    assert_eq!(states[&5], TaskState::Ready);
    assert_eq!(states[&6], TaskState::Blocked);
    assert_eq!(states[&7], TaskState::Defined);
    assert_eq!(states[&8], TaskState::Ready);
    assert_eq!(states[&9], TaskState::Blocked);
    // a section is never computed, nor a card past defined
    assert_eq!(states[&1], TaskState::Defined);
    assert_eq!(states[&4], TaskState::InProgress);
    assert_eq!(effective_state(&nodes, &nodes[4]), TaskState::Ready);
    // the stored states are untouched
    assert!(nodes.iter().skip(4).all(|n| n.state == TaskState::Defined));
}

#[test]
fn next_names_the_first_ready_card() {
    let mut root = node(1, None);
    root.kind = NodeKind::Section;
    let nodes = vec![
        root,
        task(2, TaskState::Accepted, &[]),
        task(3, TaskState::Defined, &[4]),
        task(4, TaskState::Defined, &[2]),
        task(5, TaskState::Defined, &[]),
    ];
    assert_eq!(next_ready(&nodes).map(|n| n.id), Some(4));
    assert!(next_ready(&nodes[..3]).is_none());
}
