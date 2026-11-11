//! The tests of the window's pure parts: the sums the roadmap column shows.

use super::*;

fn totals(calls: u64, cost: f64) -> Totals {
    Totals { calls, cost_usd: Some(cost), ..Totals::default() }
}

fn roadmap() -> Vec<Node> {
    ra_roadmap::markdown::parse(
        "- 1 ‹project› Rhipper {c3} [defined]\n\
         \x20   - 2 ‹milestone› Phase A {c3} [defined]\n\
         \x20       - 3 ‹task› First {c2} [accepted]\n\
         \x20       - 4 ‹task› Second {c2} [defined]\n\
         \x20   - 5 ‹task› Loose {c1} [defined]\n",
    )
    .expect("a roadmap")
}

#[test]
fn the_root_row_costs_what_the_ledger_totals() {
    let nodes = roadmap();
    // a card of each kind, a call booked to a card the roadmap lost, and one outside any card
    let by_node = vec![(Some(3), totals(2, 0.25)), (Some(5), totals(1, 0.10)), (Some(99), totals(1, 0.05)), (None, totals(1, 0.01))];
    let costs = subtree_costs(&nodes, &by_node);
    let ledger_total = by_node.iter().fold(Totals::default(), |mut acc, (_, t)| {
        acc.merge(t);
        acc
    });
    assert_eq!(costs[&1], ledger_total);
    assert_eq!(costs[&2], totals(2, 0.25));
    assert_eq!(costs[&3], totals(2, 0.25));
    assert_eq!(costs[&4], Totals::default());
    assert_eq!(costs[&5], totals(1, 0.10));
}

#[test]
fn a_diff_splits_by_file_in_the_handoffs_order() {
    let output = "diff --git a/b.rs b/b.rs\nindex 1..2 100644\n--- a/b.rs\n+++ b/b.rs\n@@ -1 +1 @@\n-old\n+new\n\
                  diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -0,0 +1 @@\n+only\n";
    let files = vec!["a.rs".to_owned(), "b.rs".to_owned(), "c.rs".to_owned()];
    let split = split_diff(output, &files);
    assert_eq!(split.len(), 3);
    assert_eq!(split[0].0, "a.rs");
    assert_eq!(split[0].1.len(), 5);
    assert_eq!(split[1].0, "b.rs");
    assert_eq!(split[1].1.len(), 7);
    // a file git has no change for keeps its header with nothing under it
    assert!(split[2].1.is_empty());
}
