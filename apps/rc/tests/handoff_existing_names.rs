//! The handoff lists up to five tracked files that already hold the card's backticked names.

use std::path::Path;
use std::process::Command;

#[allow(dead_code)]
#[path = "../src/existing_names.rs"]
mod existing_names;

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git").args(args).current_dir(dir).status().unwrap();
    assert!(status.success(), "git {args:?}");
}

fn repo(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    for (name, text) in files {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    git(dir.path(), &["add", "."]);
    dir
}

#[test]
fn a_card_name_lists_the_tracked_files_that_hold_it() {
    let dir = repo(&[("a.rs", "fn widget_count() {}"), ("b.rs", "fn other() {}")]);
    std::fs::write(dir.path().join("loose.rs"), "widget_count").unwrap();
    let files = existing_names::files_holding(dir.path(), "Change `widget_count` and run `cargo test -p rc`.");
    assert_eq!(files, vec!["a.rs".to_owned()]);
    let note = existing_names::existing_names_note(dir.path(), "Change `widget_count`.");
    assert!(note.contains("a.rs"), "{note}");
}

#[test]
fn no_more_than_five_files_are_listed() {
    let files: Vec<(String, String)> = (0..8).map(|n| (format!("f{n}.rs"), "widget_count".to_owned())).collect();
    let refs: Vec<(&str, &str)> = files.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let dir = repo(&refs);
    assert_eq!(existing_names::files_holding(dir.path(), "`widget_count`").len(), 5);
}

#[test]
fn a_card_without_a_found_name_adds_no_note() {
    let dir = repo(&[("a.rs", "fn other() {}")]);
    assert_eq!(existing_names::existing_names_note(dir.path(), "Add `missing_name`."), "");
}
