use ra_console::gotcha_edit_space::{TRAILING_SPACE_NOTE, edit_missed_message};

const BASE: &str = "an edit missed";

#[test]
fn a_replacement_ending_in_a_space_gets_the_note() {
    let message = edit_missed_message(BASE, "let x = 1; ");
    assert!(message.starts_with(BASE), "{message}");
    assert!(message.contains(TRAILING_SPACE_NOTE), "{message}");
    assert!(message.contains("non-space"), "{message}");
}

#[test]
fn a_replacement_ending_in_a_newline_gets_the_note() {
    assert!(edit_missed_message(BASE, "x\n").contains("trailing space"));
}

#[test]
fn a_replacement_ending_on_text_keeps_the_message() {
    assert_eq!(edit_missed_message(BASE, "let x = 1;"), BASE);
    assert_eq!(edit_missed_message(BASE, ""), BASE);
}
