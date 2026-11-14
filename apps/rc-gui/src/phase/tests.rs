use super::*;

fn steps(of: &[&str]) -> Vec<String> {
    of.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn the_strip_shares_sum_to_one() {
    let steps = steps(&["Read src/a.rs", "Grep fn", "Edit src/a.rs", "$ cargo test", "the change is in"]);
    let live = Live { started: 1000, stepped: 1300, number: 1, cause: None, steps: &steps };
    let strip = strip(&live, 1450);
    let sum: f32 = strip.iter().map(|(_, share)| share).sum();
    assert!((sum - 1.0).abs() < 1e-4, "{strip:?}");
    assert_eq!(strip.iter().map(|(p, _)| *p).collect::<Vec<_>>(), PHASES.to_vec());
    // the quiet since the last step is the last step's phase
    let talk = strip.iter().find(|(p, _)| *p == Phase::Talk).unwrap().1;
    let read = strip.iter().find(|(p, _)| *p == Phase::Read).unwrap().1;
    assert!(talk > read, "{strip:?}");
}

#[test]
fn a_run_without_steps_or_time_still_sums_to_one() {
    let none = Live { started: 50, stepped: 50, number: 1, cause: None, steps: &[] };
    assert_eq!(strip(&none, 50), vec![(Phase::Talk, 1.0)]);
    let steps = steps(&["Read a", "Edit a"]);
    let fresh = Live { started: 50, stepped: 50, number: 1, cause: None, steps: &steps };
    let sum: f32 = strip(&fresh, 50).iter().map(|(_, share)| share).sum();
    assert!((sum - 1.0).abs() < 1e-4);
}

#[test]
fn a_run_started_600_seconds_ago_reads_ten_minutes_whatever_the_window_uptime() {
    // the feed's start is the run's, not the window's, which came up a minute ago
    let feed = "1000\tstart #434 Phase strip · opus-5-5 · cargo test · pid 7\n";
    let feed = fold(feed);
    let begun = &feed[&434];
    let now = begun.started + 600;
    let steps = steps(&["Read a"]);
    let live = Live { started: begun.started, stepped: now - 5, number: begun.number, cause: begun.cause.clone(), steps: &steps };
    let status = status(&live, now);
    assert!(status.contains("10m00s"), "{status}");
    assert!(!status.contains("run "), "a first run has no number: {status}");
}

#[test]
fn a_rerun_after_a_failed_check_says_so() {
    let feed = "100\tstart #7 Card · opus · pid 1\n\
                200\tend ✗ #7 Card needs revision on `cargo test -p x` · $0.10\n\
                300\tstart #8 Other · opus · pid 2\n\
                400\tstart #7 Card · opus · pid 3\n";
    let runs = fold(feed);
    assert_eq!(runs[&7], Begun { started: 400, number: 2, cause: Some("check".to_owned()) });
    assert_eq!(runs[&8].cause, None);
    let live = Live { started: 400, stepped: 410, number: 2, cause: runs[&7].cause.clone(), steps: &[] };
    let status = status(&live, 420);
    assert!(status.starts_with("run 2 · after ✗ check · 20s"), "{status}");
}

#[test]
fn a_card_that_landed_comes_back_without_a_cause() {
    let feed = "100\tstart #7 Card · opus\n\
                150\treview ✗ #7 Card · needs revision · by sonnet\n\
                200\tstart #7 Card · opus\n\
                300\tlanded #7 Card · check passed\n\
                400\tstart #7 Card · opus\n";
    let runs = fold(feed);
    assert_eq!(runs[&7].number, 3);
    assert_eq!(runs[&7].cause, None);
}

#[test]
fn a_hundred_and_thirty_quiet_seconds_of_work_are_flagged() {
    let steps = steps(&["$ cargo build"]);
    let live = Live { started: 0, stepped: 1000, number: 1, cause: None, steps: &steps };
    assert_eq!(quiet(&live, 1130), Some(130));
    assert!(status(&live, 1130).contains("⚠ quiet 2m10s"));
    assert_eq!(quiet(&live, 1100), None);
    assert!(!status(&live, 1100).contains("quiet"));
}

#[test]
fn a_feed_card_is_found_by_its_run_title() {
    let feed = Feed { seen: None, runs: fold("5\tstart #12 Title · opus\n") };
    assert_eq!(feed.of_title("#12 Title · opus").map(|b| b.started), Some(5));
    assert!(feed.of_title("worker").is_none());
}
