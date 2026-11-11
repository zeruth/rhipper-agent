//! The tests of the pane's pure parts: the filter and the rows it leaves.

use super::*;

/// The text of every row the pane would draw for `nodes`, nothing folded, nothing picked.
fn tree_lines(nodes: &[Node]) -> Vec<String> {
    let (roots, shape) = shape_of(nodes);
    let chain = HashSet::new();
    roots
        .iter()
        .flat_map(|root| rows_of(root, &shape, &[], None, &chain, None, 0, 60))
        .map(|row| row.line.spans.iter().map(|s| s.content.as_ref()).collect::<String>())
        .collect()
}

#[test]
fn a_filter_leaves_the_hidden_cards_and_the_empty_structure_out_of_the_tree() {
    // the indent nests: two spaces a level
    let nodes = ra_roadmap::markdown::parse(
        "- 1 ‹project› Rhipper\n  \
         - 2 ‹milestone› Phase A\n    \
         - 3 ‹section› Done things\n      \
         - 4 ‹task› Finished [accepted]\n    \
         - 5 ‹section› Open things\n      \
         - 6 ‹task› Pending\n      \
         - 7 ‹task› Dropped [failed]\n        \
         - 8 ‹subtask› Under the dropped one\n",
    )
    .unwrap();
    assert_eq!(nodes[7].parent, Some(7), "{nodes:?}");
    let all = tree_lines(&nodes).join("\n");
    assert!(all.contains("Finished") && all.contains("Dropped"), "{all}");

    let shown = visible(&nodes, &[TaskState::Accepted, TaskState::Failed]);
    let lines = tree_lines(&shown).join("\n");
    assert!(lines.contains("Pending") && lines.contains("Open things") && lines.contains("Phase A"), "{lines}");
    // the hidden cards go, the subtask under one with them, and the section left empty
    for gone in ["Finished", "Dropped", "Under the dropped", "Done things"] {
        assert!(!lines.contains(gone), "{gone} shows:\n{lines}");
    }
    // no filter is the roadmap as it is
    assert_eq!(visible(&nodes, &[]).len(), nodes.len());
}
