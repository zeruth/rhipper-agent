//! The tests of the tree walks.

use super::*;
use crate::model::{Gambit, GambitState, NodeKind, TaskState};

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
        needs: Vec::new(),
        state: TaskState::Defined,
        assignee: None,
        complexity: 3,
        effort: crate::model::Effort::default(),
        estimate: None,
        notes: None,
        assessed: None,
        priority: None,
        absolute: false,
        evidence: None,
        saving: None,
    }
}

fn ids(nodes: &[&Node]) -> Vec<i64> {
    nodes.iter().map(|n| n.id).collect()
}

#[test]
fn is_within_follows_the_parents_up() {
    let nodes = vec![node(1, None), node(2, Some(1)), node(3, Some(2)), node(5, Some(1))];
    assert!(is_within(&nodes, 3, 1));
    assert!(is_within(&nodes, 3, 3));
    assert!(!is_within(&nodes, 5, 2));
    assert!(!is_within(&nodes, 9, 1));
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
    assert_eq!(next_ready(&nodes, &[]).map(|n| n.id), Some(4));
    assert!(next_ready(&nodes[..3], &[]).is_none());
}

fn gambit(id: u32, weight: u8, state: GambitState, focus: &[&str]) -> Gambit {
    Gambit { id, text: format!("gambit {id}"), weight, state, focus: focus.iter().map(|w| (*w).to_owned()).collect(), plan_next: false }
}

#[test]
fn the_walk_scores_the_ready_cards() {
    // under the milestone "The console": #2 first in the file, #3 matches g1 by its title and g2
    // by its files, #4 matches nothing but carries a priority; g3 matches #3 too but is parked
    let mut root = node(1, None);
    root.kind = NodeKind::Milestone;
    root.title = "The console".to_owned();
    let nodes = vec![
        root,
        Node { title: "Refresh the badge".to_owned(), ..task(2, TaskState::Defined, &[]) },
        Node {
            title: "A local route".to_owned(),
            files: vec!["crates/ra-providers/src/lib.rs".to_owned()],
            ..task(3, TaskState::Defined, &[])
        },
        Node { title: "Ship it".to_owned(), priority: Some(5), ..task(4, TaskState::Defined, &[]) },
    ];
    let gambits = vec![
        gambit(1, 3, GambitState::Active, &["Local"]),
        gambit(2, 2, GambitState::Active, &["ra-providers"]),
        gambit(3, 5, GambitState::Parked, &["local"]),
        gambit(4, 1, GambitState::Active, &["console"]),
    ];
    // the matching card comes before the earlier one; the priority before both
    assert_eq!(next_ready(&nodes[..3], &gambits).map(|n| n.id), Some(3));
    assert_eq!(next_ready(&nodes, &gambits).map(|n| n.id), Some(4));
    let scores = ranked(&nodes, &gambits);
    assert_eq!(scores.iter().map(|s| s.node.id).collect::<Vec<_>>(), vec![4, 3, 2]);
    // the milestone's title matches every card through g4; the parked gambit moves nothing
    assert_eq!((scores[1].priority, scores[1].weight), (0, 6));
    assert_eq!(scores[1].gambits.iter().map(|g| g.id).collect::<Vec<_>>(), vec![1, 2, 4]);
    assert_eq!((scores[2].priority, scores[2].weight), (0, 1));
    let line = scores[1].line();
    assert!(line.starts_with("#3 A local route") && line.contains("g1 gambit 1 {w3}") && line.contains("+6"), "{line}");
    // without gambits the file's order stands
    assert_eq!(ranked(&nodes[..3], &[]).iter().map(|s| s.node.id).collect::<Vec<_>>(), vec![2, 3]);
}

/// A card with everything a card needs: the lint has nothing to say about it.
fn whole(id: i64, state: TaskState, depends_on: &[i64]) -> Node {
    Node {
        title: format!("card {id}"),
        objective: Some("does a thing".to_owned()),
        acceptance: Some("`cargo test -p x` passes".to_owned()),
        files: vec!["src/lib.rs".to_owned()],
        ..task(id, state, depends_on)
    }
}

#[test]
fn the_lint_trips_each_rule_once() {
    let mut root = node(1, None);
    root.kind = NodeKind::Section;
    root.title = "the section".to_owned();
    // #2 is clean; #3 has a template objective; #4 no acceptance; #5 no files; #6 waits on a
    // missing card; #7 and #8 wait on each other; #9 has complexity 0
    let nodes = vec![
        root,
        whole(2, TaskState::Defined, &[]),
        Node { objective: Some("<the objective, one line>".to_owned()), ..whole(3, TaskState::Defined, &[]) },
        Node { acceptance: None, ..whole(4, TaskState::Defined, &[]) },
        Node { files: Vec::new(), ..whole(5, TaskState::Defined, &[]) },
        whole(6, TaskState::Defined, &[99]),
        whole(7, TaskState::Defined, &[8]),
        whole(8, TaskState::Defined, &[7]),
        Node { complexity: 0, ..whole(9, TaskState::Defined, &[]) },
    ];
    let lines = lint(&nodes);
    assert_eq!(lines.len(), 7, "{lines:#?}");
    assert!(lines[0].starts_with("#3:") && lines[0].contains("objective"), "{}", lines[0]);
    assert!(lines[1].starts_with("#4:") && lines[1].contains("acceptance"), "{}", lines[1]);
    assert!(lines[2].starts_with("#5:") && lines[2].contains("files"), "{}", lines[2]);
    assert!(lines[3].starts_with("#6:") && lines[3].contains("#99"), "{}", lines[3]);
    assert!(lines[4].starts_with("#7:") && lines[4].contains("waits-on"), "{}", lines[4]);
    assert!(lines[5].starts_with("#8:") && lines[5].contains("waits-on"), "{}", lines[5]);
    assert!(lines[6].starts_with("#9:") && lines[6].contains("complexity 0"), "{}", lines[6]);
}

#[test]
fn definition_of_done_reports_an_acceptance_without_a_command_and_one_that_asks_a_person() {
    let nodes = vec![
        whole(2, TaskState::Defined, &[]),
        Node { acceptance: Some("the thing is done".to_owned()), ..whole(3, TaskState::Defined, &[]) },
        Node { acceptance: Some("`cargo test -p x` passes and CI run is green".to_owned()), ..whole(4, TaskState::Defined, &[]) },
        Node {
            acceptance: Some("`cargo test -p x` passes\nperson: confirms it on the emulator".to_owned()),
            ..whole(5, TaskState::Defined, &[])
        },
        Node { acceptance: Some("the thing is done".to_owned()), ..whole(6, TaskState::Accepted, &[]) },
    ];
    let lines = lint(&nodes);
    assert_eq!(lines.len(), 2, "{lines:#?}");
    assert!(lines[0].starts_with("#3:") && lines[0].contains("no command"), "{}", lines[0]);
    assert!(lines[1].starts_with("#4:") && lines[1].contains("`person:`"), "{}", lines[1]);
}

#[test]
fn a_card_without_files_takes_the_nearest_ancestors_and_the_lint_accepts_them() {
    let mut root = node(1, None);
    root.kind = NodeKind::Milestone;
    root.title = "the milestone".to_owned();
    root.files = vec!["crates/a/".to_owned()];
    let mut section = node(2, Some(1));
    section.kind = NodeKind::Section;
    section.title = "the section".to_owned();
    section.files = vec!["crates/a/src/lib.rs".to_owned()];
    let own = Node { parent: Some(2), ..whole(3, TaskState::Defined, &[]) };
    let bare = Node { parent: Some(2), files: Vec::new(), ..whole(4, TaskState::Defined, &[]) };
    let nodes = vec![root, section, own, bare];
    assert_eq!(files_of(&nodes, &nodes[2]), (&["src/lib.rs".to_owned()][..], None));
    let (files, from) = files_of(&nodes, &nodes[3]);
    assert_eq!(files, &["crates/a/src/lib.rs".to_owned()][..]);
    assert_eq!(from.map(|n| n.id), Some(2));
    assert!(lint(&nodes).is_empty(), "{:#?}", lint(&nodes));
}

#[test]
fn a_clean_roadmap_lints_to_nothing_and_a_done_card_is_left_alone() {
    let mut root = node(1, None);
    root.kind = NodeKind::Section;
    root.title = "the section".to_owned();
    // a section carries no acceptance or files, and an accepted card is not asked for them
    let nodes = vec![
        root,
        whole(2, TaskState::Defined, &[]),
        Node { acceptance: None, files: Vec::new(), ..whole(3, TaskState::Accepted, &[]) },
        task(4, TaskState::Cancelled, &[]),
    ];
    assert!(lint(&nodes).is_empty(), "{:#?}", lint(&nodes));
}

#[test]
fn placeholder_text_is_a_bracketed_hint_or_a_bare_tbd() {
    assert!(is_placeholder("<the acceptance>"));
    assert!(is_placeholder("TBD"));
    assert!(is_placeholder("todo."));
    assert!(is_placeholder("  "));
    assert!(!is_placeholder("the tests pass"));
    assert!(!is_placeholder("<rc> prints the ids"));
}
