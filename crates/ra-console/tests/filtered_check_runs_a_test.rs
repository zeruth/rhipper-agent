//! A filtered `cargo test` check keeps the result lines of the binaries that ran a test, and
//! fails when none did.

use ra_console::agent::ran_of;

const OUTPUT: &str = "running 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 11 filtered out\n\n\
running 1 test\n.\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out\n\n\
running 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 11 filtered out\n";

#[test]
fn the_kept_output_is_the_binary_that_ran_the_test() {
    let kept = ran_of(OUTPUT).unwrap();
    assert_eq!(kept, "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out");
}

#[test]
fn a_filter_that_matched_no_test_fails() {
    let output = "running 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 11 filtered out\n";
    let error = ran_of(output).unwrap_err().to_string();
    assert!(error.contains("the filter matched no test"), "{error}");
}

#[test]
fn an_output_without_result_lines_is_kept() {
    assert_eq!(ran_of("ok\n").unwrap(), "ok\n");
}
