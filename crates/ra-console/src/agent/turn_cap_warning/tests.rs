use super::*;

#[test]
fn the_warning_comes_once_five_turns_before_the_cap() {
    let dir = tempfile::tempdir().unwrap();
    let said: Vec<Option<String>> = (0..10).map(|_| warning_at(dir.path(), 8)).collect();
    assert_eq!(said.iter().filter(|s| s.is_some()).count(), 1);
    let at = said.iter().position(Option::is_some).unwrap();
    assert_eq!(at, 2, "the third call is turn 3 of 8");
    assert!(said[at].as_deref().unwrap().contains("write your report"));
}

#[test]
fn a_cap_within_the_margin_gets_no_warning() {
    let dir = tempfile::tempdir().unwrap();
    assert!((0..10).all(|_| warning_at(dir.path(), 5).is_none()));
}
