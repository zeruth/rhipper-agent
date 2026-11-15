//! The `edit-missed` gotcha's message names the tool error it answers.

use ra_providers::claude_code::edit_missed_for_error;

const BASE: &str = "an edit missed";

#[test]
fn an_unread_file_is_told_to_be_read_first() {
    let message = edit_missed_for_error(BASE, "File has not been read yet. Read it first before writing to it.");
    assert!(message.contains("Read the file first"), "{message}");
}

#[test]
fn several_matches_are_told_to_be_made_unique() {
    let message = edit_missed_for_error(BASE, "Found 3 matches of the string to replace, but replace_all is false");
    assert!(message.contains("surrounding lines") && message.contains("replace_all"), "{message}");
}

#[test]
fn a_missing_string_is_told_to_be_quoted_exactly() {
    let message = edit_missed_for_error(BASE, "String to replace not found in file.");
    assert!(message.contains("Read the lines again and quote them exactly"), "{message}");
}

#[test]
fn another_error_keeps_the_general_message() {
    assert_eq!(edit_missed_for_error(BASE, "no changes to make"), BASE);
}
