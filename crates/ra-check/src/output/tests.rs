use super::*;

const PASS: &str = "   Compiling x v0.1.0\n    Finished `test` profile\n     Running unittests src/lib.rs\n\nrunning 3 tests\ntest a ... ok\ntest b ... ok\ntest c ... ok\n\ntest result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n\n   Doc-tests x\n\nrunning 2 tests\ntest result: ok. 2 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.10s\n";

const FAILURE: &str = "running 2 tests\ntest good ... ok\ntest bad::case ... FAILED\n\nfailures:\n\nfailures:\n    bad::case\n\ntest result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n\nerror: test failed, to rerun pass `-p x --lib`\n";

const COMPILE: &str = "   Compiling x v0.1.0\nerror[E0425]: cannot find value `y` in this scope\n --> src/lib.rs:3:5\n\nerror: could not compile `x` (lib test) due to 1 previous error\n";

const FILTERED: &str =
    "running 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 12 filtered out; finished in 0.00s\n";

#[test]
fn a_passing_run_sums_every_test_binary() {
    let ran = parse(PASS);
    assert_eq!(ran, Ran { tests: 6, failed: vec![], compile_error: None });
}

#[test]
fn a_failing_run_names_the_failed_test_once() {
    let ran = parse(FAILURE);
    assert_eq!(ran.tests, 2);
    assert_eq!(ran.failed, vec!["bad::case".to_owned()]);
    assert_eq!(ran.compile_error, None);
}

#[test]
fn a_compile_error_is_the_first_error_line() {
    let ran = parse(COMPILE);
    assert_eq!(ran.tests, 0);
    assert_eq!(ran.compile_error.as_deref(), Some("error[E0425]: cannot find value `y` in this scope"));
}

#[test]
fn a_filtered_run_of_nothing_ran_no_tests() {
    assert_eq!(parse(FILTERED), Ran::default());
}

#[test]
fn gradle_failures_read_as_class_dot_method() {
    let ran = parse("FooTest > bar FAILED\n    java.lang.AssertionError\n");
    assert_eq!(ran.failed, vec!["FooTest.bar".to_owned()]);
}

const DIFF: &str = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,12 @@\n fn old() {}\n+\n+#[test]\n+fn first_new() {}\n+\n+#[test]\n+fn second_new() {}\n+\n+fn helper() {}\n";

#[test]
fn new_tests_name_the_added_tests_and_which_the_run_printed() {
    let added = new_tests(DIFF);
    assert_eq!(added, vec!["first_new".to_owned(), "second_new".to_owned()]);
    let ran = ran_of(&added, "running 1 test\ntest tests::first_new ... ok\n");
    assert_eq!(ran, vec!["first_new".to_owned()]);
}

#[test]
fn quiet_cargo_failures_read_by_name() {
    let ran = parse("bad::case --- FAILED\n");
    assert_eq!(ran.failed, vec!["bad::case".to_owned()]);
}
