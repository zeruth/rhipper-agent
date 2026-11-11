//! The tests of the pure helpers of the UI: the wrapping, the badges and the marquee.

use super::*;

#[test]
fn wrap_breaks_on_spaces_and_hard_splits_long_words_on_characters() {
    assert_eq!(wrap("one two three", 7), ["one two", "three"]);
    // a word of box-drawing characters is split per character, never inside one
    assert_eq!(wrap("╭────────╮", 4), ["╭───", "────", "─╮"]);
    assert_eq!(wrap("", 4), [""]);
}

#[test]
fn compact_counts_read_like_a_badge() {
    assert_eq!(compact(950), "950");
    assert_eq!(compact(12_345), "12.3k");
    assert_eq!(compact(1_234_567), "1.2M");
}

#[test]
fn dollars_show_cents_a_floor_or_a_dash() {
    assert_eq!(dollars(None), "$—");
    assert_eq!(dollars(Some(0.0)), "$0.00");
    assert_eq!(dollars(Some(0.004)), "<$0.01");
    assert_eq!(dollars(Some(1.236)), "$1.24");
}

#[test]
fn the_marquee_windows_a_long_name_and_leaves_a_short_one() {
    assert_eq!(marquee("short", 10, 3), "short");
    assert_eq!(marquee("abcdef", 4, 0), "abcd");
    assert_eq!(marquee("abcdef", 4, 2), "cdef");
    // the window wraps through the three-space gap back to the start
    assert_eq!(marquee("abcdef", 4, 5), "f   ");
    assert_eq!(marquee("abcdef", 4, 8), " abc");
    assert_eq!(marquee("abcdef", 4, 9), "abcd");
}

#[test]
fn a_run_bubble_keeps_its_height_live_and_shrinks_when_done() {
    let text = |line: &Line<'_>| line.spans.iter().map(|s| s.content.as_ref()).collect::<String>();
    let mut run = crate::logs::Run {
        steps: (1..=9).map(|i| format!("Read file{i}.rs")).collect(),
        started: std::time::Instant::now(),
        took: None,
        outcome: None,
        height: 4,
    };
    let rows = run_rows("#18 Bubbles · opus-5-5", &run, 60, 0, 7);
    // the top border, four rows of steps, the bottom border, all one width
    assert_eq!(rows.len(), 6);
    assert!(rows.iter().all(|r| text(r).chars().count() == 60), "{:?}", rows.iter().map(text).collect::<Vec<_>>());
    assert!(text(&rows[0]).contains("#18 Bubbles"), "{}", text(&rows[0]));
    assert!(text(&rows[0]).contains("9 steps"), "{}", text(&rows[0]));
    // the oldest steps fold into one row; the newest is last
    assert!(text(&rows[1]).contains("6 earlier steps"), "{}", text(&rows[1]));
    assert!(text(&rows[4]).contains("Read file9.rs"), "{}", text(&rows[4]));

    run.steps.truncate(2);
    run.outcome = Some("✓ $0.12".into());
    run.took = Some(std::time::Duration::from_secs(75));
    let rows = run_rows("#18 Bubbles · opus-5-5", &run, 60, 0, 7);
    assert_eq!(rows.len(), 4);
    assert!(text(&rows[0]).contains("✓ $0.12 · 1m15s"), "{}", text(&rows[0]));
}

#[test]
fn short_title_clips_with_an_ellipsis() {
    assert_eq!(short_title("Roadmap engine", 20), "Roadmap engine");
    assert_eq!(short_title("Roadmap engine", 8), "Roadmap…");
}
