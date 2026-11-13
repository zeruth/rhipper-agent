use super::*;

#[test]
fn a_card_run_three_times_is_one_row_with_what_did_not_land() {
    let feed = "1000\tstart #235 The ledger · opus-5-5 · checks · pid 1\n\
                1600\tend ✗ #235 The ledger needs revision on opus-5-5 · turns 40 · in 900000 (90% cached) out 9000 · $2.54\n\
                1601\trerun #235 The ledger · the check failed · the walk takes it up again on its session (1 of 2)\n\
                2000\tstart #235 The ledger · opus-5-5 · checks · pid 2\n\
                2300\tend ✓ #235 The ledger implemented on opus-5-5 · turns 9 · in 100000 out 2000 · $0.31\n\
                2310\treview ✗ #235 The ledger needs revision (substantive) · 2 point(s) by m · in 100 out 10 · $0.10\n\
                3000\tstart #235 The ledger · opus-5-5 · checks · pid 3\n\
                3200\tend ✓ #235 The ledger implemented on opus-5-5 · turns 5 · in 50000 out 1000 · $0.25\n\
                3210\treview ✓ #235 The ledger accepted by m · in 100 out 10 · $0.05\n\
                3300\tlanded #235 The ledger · squashed\n";
    let rows = rows_of(feed, 0, 0, "agent");
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!((row.id, row.title.as_str(), row.group, row.runs), (235, "The ledger", Group::Landed, 3));
    assert!((row.not_landed - 2.95).abs() < 1e-9, "{}", row.not_landed);
    assert!((row.cost - 3.25).abs() < 1e-9, "{}", row.cost);
    assert_eq!(row.wall, 600 + 300 + 200);
    assert_eq!(row.at, 3300);
}

#[test]
fn a_card_with_only_a_failed_run_is_sent_back_with_its_cause() {
    let feed = "100\tstart #236 The feed · opus-5-5 · checks · pid 1\n\
                400\tend ✗ #236 The feed failed: the harness answered 529 overloaded\n";
    let rows = rows_of(feed, 0, 1, "agent");
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.group, Group::SentBack);
    assert_eq!(row.verdict, "failed");
    assert_eq!(row.sentence, "the harness answered 529 overloaded");
    assert_eq!((row.runs, row.wall, row.tab), (1, 300, 1));
}

#[test]
fn the_span_drops_older_runs() {
    let feed = "100\tstart #7 Old · m\n\
                200\tend ✓ #7 Old implemented on m · $1.00\n\
                5000\tstart #8 Both · m\n\
                5100\tend ✗ #8 Both needs revision on m · $0.40\n\
                9000\tstart #8 Both · m\n\
                9100\tend ✓ #8 Both implemented on m · $0.20\n\
                9110\treview ✓ #8 Both accepted by m · $0.05\n";
    let rows = rows_of(feed, 6000, 0, "p");
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![8]);
    let row = &rows[0];
    assert_eq!(row.runs, 1);
    assert!((row.cost - 0.25).abs() < 1e-9, "{}", row.cost);
    assert_eq!((row.group, row.verdict.as_str(), row.sentence.as_str()), (Group::Yours, "accepted", "accepted by m"));
}

#[test]
fn the_reviewers_first_line_and_the_waits_reason_are_the_sentences() {
    let feed = "100\tstart #1 One · m\n\
                200\tend ✓ #1 One implemented on m · $0.10\n\
                210\treview ✗ #1 One needs revision (substantive) · 5 point(s) by m · in 100 out 10 · $0.10\n\
                300\tstart #2 Two · m\n\
                400\tend ✗ #2 Two needs revision on m · $0.10\n\
                401\twaits #2 Two · the check failed twice · waits for the person: `rc set 2 defined` re-queues it\n";
    let rows = rows_of(feed, 0, 0, "p");
    let one = rows.iter().find(|r| r.id == 1).unwrap();
    assert_eq!((one.group, one.sentence.as_str()), (Group::SentBack, "needs revision (substantive) · 5 point(s) by m"));
    let two = rows.iter().find(|r| r.id == 2).unwrap();
    assert_eq!((two.group, two.verdict.as_str(), two.sentence.as_str()), (Group::Yours, "waits", "the check failed twice"));
}

fn row(tab: usize, id: i64, group: Group, at: u64) -> Row {
    Row {
        tab,
        project: format!("p{tab}"),
        id,
        title: format!("card {id}"),
        group,
        verdict: String::new(),
        sentence: String::new(),
        runs: 1,
        cost: 0.0,
        not_landed: 0.0,
        wall: 0,
        at,
    }
}

#[test]
fn the_rows_group_landed_sent_back_and_yours_newest_first() {
    let mut rows = vec![
        row(0, 1, Group::Yours, 500),
        row(1, 2, Group::Landed, 100),
        row(0, 3, Group::Landed, 300),
        row(1, 4, Group::SentBack, 200),
        row(0, 5, Group::Landed, 50),
    ];
    order(&mut rows, false);
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![3, 2, 5, 4, 1]);
    order(&mut rows, true);
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![3, 5, 2, 4, 1]);
}
