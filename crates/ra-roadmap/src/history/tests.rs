//! The tests of the history: the stamp, the line and the file.

use super::*;
use tempfile::tempdir;

#[test]
fn a_stamp_reads_back_as_the_seconds_it_came_from() {
    for secs in [0, 86_399, 951_782_400, 1_762_891_203, 4_102_444_800] {
        let stamp = format_time(secs);
        assert_eq!(parse_time(&stamp), Some(secs), "{stamp}");
    }
    assert_eq!(format_time(1_762_891_203), "2025-11-11T20:00:03Z");
    assert_eq!(format_time(951_782_400), "2000-02-29T00:00:00Z");
    assert_eq!(parse_time("2025-13-01T00:00:00Z"), None);
    assert_eq!(parse_time("yesterday"), None);
}

#[test]
fn a_line_parses_back_to_its_change() {
    let change = Change { at: 1_762_891_203, id: 12, from: TaskState::Defined, to: TaskState::InProgress, by: Source::Cli };
    assert_eq!(change.line(), "2025-11-11T20:00:03Z #12 defined -> in_progress by cli");
    assert_eq!(Change::parse(&change.line()), Some(change));
    assert_eq!(Change::parse("2025-11-11T20:00:03Z #12 defined -> in_progress by nobody"), None);
    assert_eq!(Change::parse(""), None);
}

#[test]
fn the_file_keeps_the_changes_in_order_and_filters_by_card() {
    let dir = tempdir().unwrap();
    let log = dir.path().join("var").join("history.log");
    assert!(read(&log, None).unwrap().is_empty());
    let first = Change { at: 10, id: 2, from: TaskState::Defined, to: TaskState::InProgress, by: Source::Run };
    let other = Change { at: 11, id: 3, from: TaskState::Defined, to: TaskState::Accepted, by: Source::Window };
    let second = Change { at: 12, id: 2, from: TaskState::InProgress, to: TaskState::Implemented, by: Source::Run };
    for change in [&first, &other, &second] {
        append(&log, change).unwrap();
    }
    assert_eq!(read(&log, None).unwrap().len(), 3);
    assert_eq!(read(&log, Some(2)).unwrap(), vec![first, second]);
}

#[test]
fn an_age_reads_in_the_largest_unit_that_fits() {
    assert_eq!(age(30), "now");
    assert_eq!(age(180), "3m");
    assert_eq!(age(7500), "2h 05m");
    assert_eq!(age(356_400), "4d 3h");
}
