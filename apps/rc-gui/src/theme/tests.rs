use super::*;

#[test]
fn a_theme_is_found_by_a_loose_name() {
    assert_eq!(by_name("daybreak").map(|t| t.name), Some("daybreak"));
    assert_eq!(by_name("Blood Moon").map(|t| t.name), Some("blood-moon"));
    assert_eq!(by_name("blood-moon").map(|t| t.name), Some("blood-moon"));
    assert!(by_name("midnight").is_none());
    assert!(by_name("").is_none());
}

#[test]
fn the_default_theme_is_blood_moon_and_every_theme_has_its_own_name() {
    assert_eq!(ALL[0].name, "blood-moon");
    let mut names: Vec<&str> = ALL.iter().map(|t| t.name).collect();
    names.dedup();
    assert_eq!(names.len(), ALL.len());
}
