//! A card the reviewer sent back with the same points three times, or with any points four times,
//! is held for the person: the verdicts' points hashes are compared across the card's handoffs.

use ra_context::{Handoff, POINTS_MARK, TaskContext};

fn rejected(hash: &str) -> Handoff {
    Handoff {
        when: 100,
        model: "w".into(),
        outcome: "needs revision (substantive)".into(),
        check: "check passed: cargo test -q · exit code 0".into(),
        verdict: Some(format!("needs revision (substantive) · 1 point(s) by m{POINTS_MARK}{hash}")),
        changed: vec!["src/a.rs".into()],
        report: "wrote src/a.rs".into(),
        warnings: Vec::new(),
        read: Vec::new(),
        failures: Vec::new(),
    }
}

fn after(hashes: &[&str]) -> TaskContext {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut task = TaskContext::open(dir.path(), 7);
    for hash in hashes {
        task.record(rejected(hash));
    }
    task
}

#[test]
fn the_third_verdict_with_a_repeated_hash_holds_the_card() {
    let hold = after(&["aaaa", "bbbb", "aaaa"]).review_hold();
    assert!(hold.is_some_and(|why| why.contains("same points")));
}

#[test]
fn three_verdicts_with_different_points_do_not_hold_the_card() {
    assert_eq!(after(&["aaaa", "bbbb", "cccc"]).review_hold(), None);
}

#[test]
fn two_verdicts_with_the_same_points_do_not_hold_the_card() {
    assert_eq!(after(&["aaaa", "aaaa"]).review_hold(), None);
}

#[test]
fn the_fourth_verdict_of_any_kind_holds_the_card() {
    assert!(after(&["aaaa", "bbbb", "cccc", "dddd"]).review_hold().is_some());
}
