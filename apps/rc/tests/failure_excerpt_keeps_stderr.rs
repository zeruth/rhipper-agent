//! A failed check's excerpt keeps the last 20 lines a test printed before its panic.

#[test]
fn failure_excerpt_keeps_the_stderr_lines_before_the_panic() {
    let mut output = String::from("running 1 test\ntest t ... FAILED\n\nfailures:\n\n---- t stdout ----\n");
    for i in 0..25 {
        output.push_str(&format!("line {i}\n"));
    }
    output.push_str("ra: exception reading 0x10\nthe last functions entered: f, g\n");
    output.push_str("thread 't' panicked at src/a.rs:1:1:\nboom\n\nfailures:\n    t\n\ntest result: FAILED. 0 passed; 1 failed\n");
    let failures = ra_context::failures_of(&output);
    assert_eq!(failures.len(), 1, "{failures:?}");
    let lines: Vec<&str> = failures[0].lines().collect();
    assert_eq!(lines[0], "t");
    assert_eq!(lines[1], "line 7", "{lines:?}");
    assert!(lines.contains(&"ra: exception reading 0x10"), "{lines:?}");
    assert!(lines.contains(&"the last functions entered: f, g"), "{lines:?}");
    assert!(!lines.contains(&"line 6"), "{lines:?}");
    assert_eq!(lines.last(), Some(&"boom"));
}
