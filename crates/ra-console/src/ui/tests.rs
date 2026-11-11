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
fn short_title_clips_with_an_ellipsis() {
    assert_eq!(short_title("Roadmap engine", 20), "Roadmap engine");
    assert_eq!(short_title("Roadmap engine", 8), "Roadmap…");
}
