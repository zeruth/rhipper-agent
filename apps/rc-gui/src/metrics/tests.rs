use std::path::Path;

use super::*;

/// The value and the shown text of the metric named `name` on the latest walk.
fn latest(metrics: &[Metric], name: &str) -> (f64, String) {
    let metric = metrics.iter().find(|m| m.name == name).expect("the metric is in the row");
    let value = metric.current().expect("the latest walk has a value");
    (value, (metric.format)(value))
}

#[test]
fn a_walk_of_seven_runs_over_four_cards_with_three_reruns_reads_43_percent() {
    let feed = "100\tstart #1 A · m\n\
                110\tend ✗ #1 A needs revision on m · turns 2 · in 1000 out 10 · $0.10\n\
                120\tstart #2 B · m\n\
                130\tend ✓ #2 B implemented on m · turns 2 · in 1000 out 10 · $0.10\n\
                131\treview ✗ #2 B needs revision (substantive) · 2 point(s) by m · in 100 out 10 · $0.01\n\
                140\tstart #3 C · m\n\
                150\tend ✓ #3 C implemented on m · turns 2 · in 1000 out 10 · $0.10\n\
                151\t#3 not landed: main moved; sent back to its worker to merge the project's branch in and resolve them\n\
                160\tstart #4 D · m\n\
                170\tend ✓ #4 D implemented on m · turns 2 · in 1000 out 10 · $0.10\n\
                171\treview ✓ #4 D accepted by m · in 100 out 10 · $0.01\n\
                172\tlanded #4 D · squashed\n\
                180\tstart #1 A · m\n\
                190\tstart #2 B · m\n\
                200\tstart #3 C · m\n\
                210\twalk ran 7 card(s) · no card is ready\n";
    let walks = fold(Path::new("agent"), feed);
    assert_eq!(walks.len(), 1);
    let walk = &walks[0];
    assert_eq!((walk.runs, walk.reruns), (7, 3));
    assert_eq!(walk.causes.get("check"), Some(&1));
    assert_eq!(walk.causes.get("review"), Some(&1));
    assert_eq!(walk.causes.get("conflict"), Some(&1));
    assert_eq!((walk.first_runs, walk.first_run_landed), (4, 1));
    let metrics = metrics(&walks, None);
    assert_eq!(latest(&metrics, "RERUN RATE").1, "43%");
    assert_eq!(latest(&metrics, "FIRST-RUN PASS").1, "25%");
}

#[test]
fn a_review_is_counted_once_so_two_of_three_reads_67_percent() {
    // the end line after a failed review says needs revision too; it is not a second verdict
    let feed = "100\tstart #1 A · m\n\
                110\treview ✗ #1 A needs revision (substantive) · 2 point(s) by opus · in 100 out 10 · $0.01\n\
                120\tend ✗ #1 A needs revision on m · turns 2 · in 1000 out 10 · $0.10\n\
                130\tstart #2 B · m\n\
                140\treview ✓ #2 B accepted by opus · in 100 out 10 · $0.01\n\
                150\tend ✓ #2 B implemented on m · turns 2 · in 1000 out 10 · $0.10\n\
                160\tstart #3 C · m\n\
                170\treview ✓ #3 C accepted by sonnet · in 100 out 10 · $0.01\n\
                180\tend ✓ #3 C implemented on m · turns 2 · in 1000 out 10 · $0.10\n";
    let walks = fold(Path::new("agent"), feed);
    assert_eq!((walks[0].reviews, walks[0].reviews_passed), (3, 2));
    assert_eq!(walks[0].reviewers.get("opus"), Some(&(2, 1)));
    let (value, shown) = latest(&metrics(&walks, None), "REVIEW PASS");
    assert!((value - 200.0 / 3.0).abs() < 1e-9);
    assert_eq!(shown, "67%");
}

#[test]
fn one_projects_scope_ignores_anothers_walks() {
    let mut walks = fold(
        Path::new("agent"),
        "100\tstart #1 A · m\n110\tend ✓ #1 A implemented on m · turns 1 · in 10 out 1 · $1.00\n120\tlanded #1 A · squashed\n",
    );
    walks.extend(fold(
        Path::new("asm"),
        "200\tstart #9 Z · m\n210\tend ✓ #9 Z implemented on m · turns 1 · in 10 out 1 · $5.00\n220\tlanded #9 Z · squashed\n",
    ));
    let agent = metrics(&walks, Some(Path::new("agent")));
    let cost = agent.iter().find(|m| m.name == "COST / LANDED").expect("the tile");
    assert_eq!(cost.values, vec![Some(1.0)]);
    let all = metrics(&walks, None);
    let cost = all.iter().find(|m| m.name == "COST / LANDED").expect("the tile");
    assert_eq!(cost.values, vec![Some(1.0), Some(5.0)]);
}

#[test]
fn three_dollars_eight_over_four_landed_is_77_cents() {
    let feed = "100\tstart #1 A · m\n110\tend ✓ #1 A implemented on m · turns 1 · in 10 out 1 · $1.00\n115\tlanded #1 A · squashed\n\
                120\tstart #2 B · m\n130\tend ✓ #2 B implemented on m · turns 1 · in 10 out 1 · $0.50\n135\tlanded #2 B · squashed\n\
                140\tstart #3 C · m\n150\tend ✓ #3 C implemented on m · turns 1 · in 10 out 1 · $0.80\n155\tlanded #3 C · squashed\n\
                160\tstart #4 D · m\n170\tend ✓ #4 D implemented on m · turns 1 · in 10 out 1 · $0.70\n\
                171\treview ✓ #4 D accepted by m · in 10 out 1 · $0.08\n175\tlanded #4 D · squashed\n";
    let walks = fold(Path::new("agent"), feed);
    assert!((walks[0].usd - 3.08).abs() < 1e-9);
    let (value, shown) = latest(&metrics(&walks, None), "COST / LANDED");
    assert!((value - 0.77).abs() < 1e-9);
    assert_eq!(shown, "$0.77");
}

#[test]
fn a_title_holding_in_keeps_its_input() {
    let feed = "100\tstart #5 The row in the report · m\n\
                200\tend ✓ #5 The row in the report implemented on m · turns 4, 3 tool call(s) · in 80000 (90% cached) out 500 · read-in 2.5k, 1 files whole · $0.40 est.\n";
    let walks = fold(Path::new("agent"), feed);
    let walk = &walks[0];
    assert!((walk.run_input - 80000.0).abs() < 1e-9);
    assert!((walk.cached - 72000.0).abs() < 1e-9);
    assert!((walk.read_in - 2500.0).abs() < 1e-9);
    assert!((walk.usd - 0.40).abs() < 1e-9);
    assert_eq!(walk.per_turn, vec![20000.0]);
}

#[test]
fn a_walk_ends_on_its_line_and_a_rerun_in_the_next_walk_is_still_a_rerun() {
    let feed = "100\tstart #1 A · m\n\
                110\tend ✗ #1 A failed: the provider refused the key\n\
                120\twalk ran 1 card(s) · stopped\n\
                9000\tstart #1 A · m\n\
                9100\tend ✓ #1 A implemented on m · turns 1 · in 10 out 1 · $0.10\n\
                9110\tlanded #1 A · squashed\n";
    let walks = fold(Path::new("agent"), feed);
    assert_eq!(walks.len(), 2);
    assert_eq!((walks[1].runs, walks[1].reruns, walks[1].landed), (1, 1, 1));
    assert_eq!(walks[1].causes.get("error"), Some(&1));
    assert_eq!(walks[0].first_run_missed, vec![1]);
    assert_eq!(walks[1].first_runs, 0);
}
