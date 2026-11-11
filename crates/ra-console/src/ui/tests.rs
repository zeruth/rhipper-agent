//! The tests of the pure helpers of the UI: the wrapping, the badges and the marquee.

use super::*;

#[test]
fn wrap_breaks_on_spaces_and_hard_splits_long_words_on_characters() {
    assert_eq!(wrap("one two three", 7), ["one two", "three"]);
    // a word of box-drawing characters is split per character, never inside one
    assert_eq!(wrap("╭────────╮", 4), ["╭───", "────", "─╮"]);
    assert_eq!(wrap("", 4), [""]);
    // an indented line keeps its indentation on every row, as an outline reads
    assert_eq!(wrap("    12  pub fn one", 12), ["    12  pub", "    fn one"]);
}

#[test]
fn a_folded_entry_shows_its_header_until_it_opens() {
    let text = |line: &Line<'_>| line.spans.iter().map(|s| s.content.as_ref()).collect::<String>();
    let mut line =
        crate::logs::Line {
            kind: Kind::Folded, text: "context #30 · 2 lines\nObjective: x\n  src/lib.rs".into(), run: None, folded: true
        };
    let rows = rows_of(&line, 60, 0);
    assert_eq!(rows.len(), 3, "{:?}", rows.iter().map(text).collect::<Vec<_>>());
    assert!(text(&rows[1]).contains("▸ context #30 · 2 lines · Enter unfolds"), "{}", text(&rows[1]));
    line.folded = false;
    let rows = rows_of(&line, 60, 0);
    assert_eq!(rows.len(), 5);
    assert!(text(&rows[1]).contains("▾ context #30"), "{}", text(&rows[1]));
    assert!(text(&rows[3]).contains("│   src/lib.rs"), "{}", text(&rows[3]));
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
        folded: None,
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

#[test]
fn a_search_keeps_the_hits_with_their_ancestors_and_enter_picks_the_first() {
    // the indent nests: two spaces a level
    let nodes = ra_roadmap::markdown::parse(
        "- 1 ‹project› Rhipper\n  \
         - 2 ‹milestone› Phase A\n    \
         - 3 ‹section› The ledger\n      \
         - 4 ‹task› Record the cost\n      \
         - 5 ‹task› Show the ledger\n    \
         - 6 ‹section› The console\n      \
         - 7 ‹task› Search in the tree\n        \
         - 8 ‹subtask› The search line\n",
    )
    .unwrap();
    let titles = |rows: &[ra_roadmap::model::Node]| rows.iter().map(|n| n.title.clone()).collect::<Vec<_>>();
    // by title, case-folded: the hits and the structure above them, in roadmap order
    let rows = search_rows(&nodes, "LEDGER");
    assert_eq!(titles(&rows), ["Rhipper", "Phase A", "The ledger", "Show the ledger"]);
    // by id, with or without the hash; the children of a hit do not come along
    assert_eq!(titles(&search_rows(&nodes, "#7")), ["Rhipper", "Phase A", "The console", "Search in the tree"]);
    assert_eq!(titles(&search_rows(&nodes, " 7 ")), ["Rhipper", "Phase A", "The console", "Search in the tree"]);
    // the empty search is no search, and a miss leaves nothing
    assert_eq!(search_rows(&nodes, "  ").len(), nodes.len());
    assert!(search_rows(&nodes, "nowhere").is_empty());
    // Enter takes the first hit itself, never an ancestor kept for it
    assert_eq!(first_match(&nodes, "ledger"), Some(3));
    assert_eq!(first_match(&nodes, "search"), Some(7));
    assert_eq!(first_match(&nodes, ""), None);
}
