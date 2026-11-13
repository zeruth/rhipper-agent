use std::fs;

use ra_roadmap::missing_machinery::{identifiers, missing, report};

fn tree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/lib.rs"), "pub fn present_thing() {}").unwrap();
    dir
}

#[test]
fn a_card_naming_only_absent_machinery_is_flagged() {
    let dir = tree();
    let objective = "Extend `ghost_fn` and `Phantom::build()` with more.";
    assert_eq!(missing(objective, objective, dir.path()), vec!["ghost_fn", "build"]);
    assert!(report(objective, objective, dir.path()).unwrap().contains("`ghost_fn`"));
}

#[test]
fn a_card_naming_one_present_identifier_passes() {
    let dir = tree();
    let objective = "Call `present_thing` from `ghost_fn`.";
    assert!(missing(objective, objective, dir.path()).is_empty());
}

#[test]
fn a_starts_from_nothing_line_silences_the_check() {
    let dir = tree();
    let objective = "Add `ghost_fn`.";
    let text = "Add `ghost_fn`.\nstarts-from: nothing";
    assert!(missing(objective, text, dir.path()).is_empty());
}

#[test]
fn commands_and_paths_are_not_identifiers() {
    assert_eq!(identifiers("Run `cargo test -p x` in `crates/a/src/b.rs` for `Name`."), vec!["Name"]);
}

#[test]
fn a_card_naming_nothing_passes() {
    let dir = tree();
    assert!(missing("Plain prose.", "Plain prose.", dir.path()).is_empty());
}
