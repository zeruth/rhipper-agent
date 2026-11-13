use super::*;

#[test]
fn runs_ended_before_the_mark_are_not_counted() {
    let feed = "100\tstart #1 A · m\n\
                110\tend ✓ #1 A implemented on m · turns 2 · $0.10\n\
                120\tlanded #1 A · squashed\n\
                130\tstart #2 B · m\n\
                140\tend ✗ #2 B needs revision on m · turns 2 · $0.20\n\
                200\tstart #3 C · m\n\
                210\tend ✓ #3 C implemented on m · turns 2 · $0.30\n\
                220\tlanded #3 C · squashed\n";
    let since = count(feed, 150, 0);
    assert_eq!(since.landed, 1);
    assert_eq!(since.landed_cards, vec![3]);
    assert!((since.landed_usd - 0.30).abs() < 1e-9);
    assert_eq!(since.failed_first, 0);
    let all = count(feed, 0, 0);
    assert_eq!((all.landed, all.failed_first), (2, 1));
}

#[test]
fn a_run_that_started_before_the_mark_and_ended_after_it_counts() {
    let feed = "100\tstart #1 A · m\n\
                210\tend ✓ #1 A implemented on m · $0.40\n\
                220\tlanded #1 A · squashed\n";
    assert_eq!(count(feed, 150, 0).landed, 1);
}

#[test]
fn a_first_run_failure_adds_its_cost() {
    let feed = "200\tstart #5 E · m\n\
                210\tend ✓ #5 E implemented on m · turns 2 · $1.25\n\
                211\treview ✗ #5 E needs revision (substantive) · 2 point(s) by m · $0.05\n\
                220\tstart #5 E · m\n\
                230\tend ✗ #5 E failed: check · $2.00\n";
    let since = count(feed, 150, 0);
    assert_eq!(since.failed_first, 1, "the rerun's failure is not a first run's");
    assert_eq!(since.failed_cards, vec![5]);
    assert!((since.failed_usd - 1.30).abs() < 1e-9, "the run and its review: {}", since.failed_usd);
}

#[test]
fn a_card_not_landed_on_a_moved_main_counts_a_conflict() {
    let feed = "200\tstart #7 G · m\n\
                210\tend ✓ #7 G implemented on m · $0.10\n\
                211\t#7 not landed: main moved; sent back to its worker\n";
    let since = count(feed, 150, 0);
    assert_eq!((since.conflicts, since.conflict_cards.clone()), (1, vec![7]));
}

#[test]
fn what_is_yours_counts_whatever_the_mark() {
    let feed = "100\tstart #1 A · m\n110\tend ✗ #1 A failed: x · $0.10\n";
    let since = count(feed, u64::MAX, 3);
    assert_eq!(since.yours, 3);
    assert_eq!(since.failed_first, 0);
    assert!(!since.quiet());
}

#[test]
fn planned_cards_after_the_mark_are_counted() {
    let feed = "100\tadded 4 planned card(s) under #9 by m · $0.10\n\
                200\tlook-back pass after 10 landed cards: added 3 planned card(s) under #9 by m · $0.20; archived\n";
    assert_eq!(count(feed, 150, 0).planned, 3);
}

#[test]
fn the_mark_survives_a_round_trip() {
    let root = std::env::temp_dir().join(format!("rc-gui-since-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(seen(&root), 0, "no mark reads as never looked");
    mark(&root, 1_794_563_367).expect("the mark is written");
    assert_eq!(seen(&root), 1_794_563_367);
    let _ = std::fs::remove_dir_all(&root);
}
