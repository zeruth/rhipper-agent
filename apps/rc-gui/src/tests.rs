//! The tests of the window's pure parts: the sums the roadmap column shows, the diff's split,
//! the gauges' ring and graph.

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
fn idle_lines_say_how_the_walk_stopped_what_is_yours_and_what_is_next() {
    let mut nodes = roadmap();
    nodes.iter_mut().find(|n| n.id == 5).expect("card 5").depends_on = vec![4];
    let feed = vec![
        "1000\tstart #4 Second".to_owned(),
        "1100\twalk ran 1 card · stopped: #4 needs revision".to_owned(),
        "1200\tnote unrelated".to_owned(),
    ];
    let end = walk_end_of(&feed);
    assert_eq!(end, Some((Some(1100), Some("#4 needs revision".to_owned()))));
    let waiting = vec![ra_console::hub::Waiting {
        id: 4,
        title: "Second".to_owned(),
        state: TaskState::NeedsRevision,
        reason: "the reviewer sent it back 5 times".to_owned(),
    }];
    let [walk, yours, next] = idle_lines(&nodes, &[5], &waiting, end.as_ref(), 1100 + 2 * 3600 + 300);
    assert_eq!(walk, "walk stopped 2h 05m ago: #4 needs revision");
    assert!(yours.starts_with("yours: #4 Second"), "{yours}");
    assert!(yours.ends_with("rc set 4 defined"), "{yours}");
    // the next card waits on the one sent back, and says which
    assert_eq!(next, "next: #5 Loose · behind #4");
    let [walk, yours, next] = idle_lines(&nodes, &[], &[], None, 0);
    assert_eq!(walk, "no walk has run here");
    assert_eq!(yours, "nothing waits on you");
    assert_eq!(next, "next: no card is ready");
}

#[test]
fn idle_lines_name_the_card_a_queue_row_is_behind() {
    let mut nodes = roadmap();
    nodes.iter_mut().find(|n| n.id == 5).expect("card 5").depends_on = vec![4];
    let (short, why) = queue_reason(&nodes, 5, None, None);
    assert_eq!(short, "behind #4");
    assert!(why.contains("#4 Second"), "{why}");
    let (short, why) = queue_reason(&nodes, 4, Some(3), Some(2));
    assert_eq!(short, "after #2");
    assert!(why.contains("#2"), "{why}");
    assert_eq!(queue_reason(&nodes, 4, Some(4), None).0, "running");
    let landing = ra_console::hub::Waiting {
        id: 4,
        title: "Second".to_owned(),
        state: TaskState::Implemented,
        reason: "not landed: main fails".to_owned(),
    };
    assert_eq!(landing_row(&landing, Some(0), 180), "#4 Second · landing 3m");
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

#[test]
#[allow(clippy::cast_precision_loss)]
fn a_ring_keeps_the_last_two_minutes_newest_last() {
    let mut ring = Ring::default();
    for i in 0..(cards::GAUGE_SPAN + 10) {
        ring.push(i as f32);
    }
    let samples = ring.samples();
    assert_eq!(samples.len(), cards::GAUGE_SPAN);
    assert_eq!(samples[0], 10.0);
    assert_eq!(*samples.last().unwrap(), (cards::GAUGE_SPAN + 9) as f32);
}

fn spend(at: &str, tokens: u64, cost: Option<f64>) -> ra_usage::Spend {
    ra_usage::Spend { at: at.to_owned(), tokens, cost_usd: cost }
}

#[test]
fn the_token_gauges_average_the_runs_and_read_short() {
    // two projects' session rows arrive apart and merge by time; the totals cover one run the
    // graph does not show
    let session = vec![spend("2026-11-11 10:02:00", 4_000, Some(0.04)), spend("2026-11-11 10:01:00", 2_000, Some(0.02))];
    let session_totals = Totals { calls: 3, prompt_tokens: 6_000, completion_tokens: 2_000, cost_usd: Some(0.34), ..Totals::default() };
    let all = (0..RUNS_SHOWN + 5)
        .map(|i| spend(&format!("2026-11-11 {:02}:{:02}:00", i / 60, i % 60), 1_000 + u64::try_from(i).unwrap(), None))
        .collect();
    let all_totals =
        Totals { calls: 100, prompt_tokens: 11_000_000, completion_tokens: 1_800_000, cost_usd: Some(12.4), ..Totals::default() };
    let gauges = token_gauges(session, &session_totals, all, &all_totals);
    assert_eq!(gauges.task.value, "2.7k avg · 3 runs");
    assert_eq!(gauges.task.samples, vec![0.5, 1.0]);
    assert_eq!(gauges.session.value, "8.0k · .34");
    // the running total starts over the run not shown and ends at the top
    assert_eq!(gauges.session.samples, vec![0.5, 1.0]);
    assert_eq!(gauges.all.value, "128k avg · 12.40");
    assert_eq!(gauges.all.samples.len(), RUNS_SHOWN);
    assert_eq!(*gauges.all.samples.last().unwrap(), 1.0);
    let empty = token_gauges(Vec::new(), &Totals::default(), Vec::new(), &Totals::default());
    assert_eq!(empty.task.value, "no runs yet");
    assert_eq!(empty.session.value, "0 · —");
    assert!(empty.session.samples.is_empty());
}

#[test]
fn the_token_texts_read_as_people_say_them() {
    assert_eq!(tokens_text(842), "842");
    assert_eq!(tokens_text(1_234), "1.2k");
    assert_eq!(tokens_text(9_960), "10k");
    assert_eq!(tokens_text(128_400), "128k");
    assert_eq!(tokens_text(1_250_000), "1.2M");
    assert_eq!(tokens_text(24_000_000), "24M");
    assert_eq!(cents(Some(0.34)), ".34");
    assert_eq!(cents(Some(0.006)), ".01");
    assert_eq!(cents(Some(12.4)), "12.40");
    assert_eq!(cents(None), "—");
}

#[test]
fn the_graph_puts_the_newest_sample_at_the_right_and_a_full_reading_at_the_top() {
    let plot = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(119.0, 10.0));
    let points = cards::gauge_points(&[0.0, 1.0, 0.5, 2.0], plot);
    assert_eq!(points.len(), 4);
    // the newest sits on the right edge and a young trace stretches to the left edge; a full
    // trace of GAUGE_SPAN samples would step one slot a second
    assert_eq!(points[3].x, 119.0);
    assert!((points[0].x - 0.0).abs() < 0.01, "{}", points[0].x);
    assert!((points[2].x - 119.0 * 2.0 / 3.0).abs() < 0.01, "{}", points[2].x);
    // a reading of nothing is the floor, a full one the rule at the top, more than full clamps
    assert_eq!(points[0].y, 10.0);
    assert_eq!(points[1].y, 0.0);
    assert_eq!(points[2].y, 5.0);
    assert_eq!(points[3].y, 0.0);
    assert!(cards::gauge_points(&[], plot).is_empty());
}
