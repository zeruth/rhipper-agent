//! A worker that exits without a result leaves its exit code and the last 20 lines of its
//! standard error in the failure the card's notes and the rerun's handoff quote.

use ra_providers::claude_code::crash_text;

#[test]
fn a_crash_keeps_the_exit_code_and_the_last_twenty_stderr_lines() {
    let stderr: String = (0..30).map(|i| format!("err {i}\n")).collect();
    let text = crash_text("exit code: 3", &stderr, "");
    assert!(text.starts_with("claude exited with exit code: 3 and no result: err 29"), "{text}");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[1], "stderr, last 20 lines:");
    assert_eq!(lines[2], "err 10");
    assert_eq!(lines.last(), Some(&"err 29"));
    assert_eq!(lines.len(), 22, "{text}");
}

#[test]
fn a_crash_without_stderr_names_the_last_output_line() {
    let text = crash_text("exit code: 1", "\n", "last words");
    assert_eq!(text, "claude exited with exit code: 1 and no result: last words");
}
