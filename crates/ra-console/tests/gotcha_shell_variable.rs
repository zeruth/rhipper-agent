use ra_console::gotcha_shell_variable::{ID, hit};

const REFUSAL: &str = "A variable in this command can't be checked before it runs: cargo test; echo $?";

#[test]
fn a_refused_status_variable_matches_the_gotcha_that_says_to_drop_it() {
    let hit = hit(REFUSAL).expect("the refusal matches");
    assert_eq!(hit.id, ID);
    assert!(hit.message.contains("drop `$?`"));
    assert!(hit.message.contains("tool result"));
}

#[test]
fn an_unrelated_result_matches_nothing() {
    assert!(hit("test result: ok. 3 passed").is_none());
}
