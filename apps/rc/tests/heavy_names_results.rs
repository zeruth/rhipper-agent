//! The `[heavy]` look-back line names the three largest tool results by tool, target and size.

use ra_console::retro::{HEAVY, TURN_CEILING, record_heavy};

fn largest(call: &str, chars: usize) -> (String, usize) {
    (call.to_owned(), chars)
}

#[test]
fn the_heavy_line_names_the_three_largest_tool_results() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let results = [
        largest("read_file src/big.rs", 9000),
        largest("run_command cargo test", 4000),
        largest("read_file src/mid.rs", 1200),
        largest("list_dir .", 80),
    ];
    let line = record_heavy(dir.path(), 7, TURN_CEILING + 1, &results).expect("the line is kept");
    assert!(line.starts_with(&format!("{HEAVY} #7 ")), "{line}");
    assert!(
        line.ends_with("largest tool results: read_file src/big.rs (9000 chars), run_command cargo test (4000 chars), read_file src/mid.rs (1200 chars)"),
        "{line}"
    );
    assert!(!line.contains("list_dir"), "{line}");
}

#[test]
fn a_run_with_no_results_in_sight_says_so() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let line = record_heavy(dir.path(), 8, TURN_CEILING + 1, &[]).expect("the line is kept");
    assert!(line.ends_with("out of sight"), "{line}");
}
