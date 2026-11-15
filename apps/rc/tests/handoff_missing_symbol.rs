//! The handoff flags a symbol the card names that no file defines, and names the open card
//! that adds it.

use std::path::Path;
use std::process::Command;

fn rc(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rc")).args(args).current_dir(dir).output().expect("rc runs")
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a tempdir");
    assert!(rc(dir.path(), &["init", "--empty"]).status.success());
    assert!(rc(dir.path(), &["add", "project", "Alpha"]).status.success());
    std::fs::create_dir_all(dir.path().join("src")).expect("the dir");
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn present_thing() {}\n").expect("a source");
    dir
}

fn add(dir: &Path, title: &str, objective: &str) -> String {
    let added = rc(dir, &["add", "task", title, "--parent", "1", "--objective", objective]);
    assert!(added.status.success(), "{}", String::from_utf8_lossy(&added.stderr));
    let said = String::from_utf8_lossy(&added.stdout).into_owned();
    said.split('#').nth(1).unwrap_or_else(|| panic!("no id in {said}")).chars().take_while(char::is_ascii_digit).collect()
}

fn context_of(dir: &Path, id: &str) -> String {
    let out = rc(dir, &["context", id]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn a_missing_symbol_is_flagged_with_the_open_card_that_adds_it() {
    let dir = project();
    let adder = add(dir.path(), "Add `ghost_helper` to the tree", "Write `ghost_helper`");
    let user = add(dir.path(), "Use it", "Call `ghost_helper` and `present_thing`");
    let context = context_of(dir.path(), &user);
    assert!(context.contains("Not defined in the tree: `ghost_helper`"), "{context}");
    assert!(context.contains(&format!("the open card #{adder} Add `ghost_helper` to the tree")), "{context}");
    assert!(!context.contains("`present_thing`;"), "{context}");
    assert!(!context.contains("Not defined in the tree: `present_thing`"), "{context}");
}

#[test]
fn a_missing_symbol_no_card_adds_is_flagged_alone() {
    let dir = project();
    let user = add(dir.path(), "Use it", "Call `orphan_symbol`");
    let context = context_of(dir.path(), &user);
    assert!(context.contains("Not defined in the tree: `orphan_symbol`\n"), "{context}");
    assert!(!context.contains("adds it"), "{context}");
}
