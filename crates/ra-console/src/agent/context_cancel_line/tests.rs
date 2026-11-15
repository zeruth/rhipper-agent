use super::CANCEL_LINE;

#[test]
fn context_names_cancel_deletes() {
    assert!(CANCEL_LINE.contains("Cancelling a card deletes the card and its record"));
    assert_eq!(CANCEL_LINE.lines().count(), 1);
}
