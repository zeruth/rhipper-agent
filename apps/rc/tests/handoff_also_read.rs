//! The handoff names the files that define and call the symbols a card puts in backticks,
//! past the card's own files, under an "also read" line capped at eight.

use std::path::Path;
use std::process::Command;

fn rc(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rc")).args(args).current_dir(dir).output().expect("rc runs")
}

fn write(dir: &Path, path: &str, text: &str) {
    let path = dir.join(path);
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("the dir");
    std::fs::write(path, text).expect("a source");
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a tempdir");
    assert!(rc(dir.path(), &["init", "--empty"]).status.success());
    assert!(rc(dir.path(), &["add", "project", "Alpha"]).status.success());
    dir
}

fn context_of(dir: &Path, objective: &str, files: &str) -> String {
    let added = rc(dir, &["add", "task", "Change", "--parent", "1", "--objective", objective, "--files", files]);
    assert!(added.status.success(), "{}", String::from_utf8_lossy(&added.stderr));
    let said = String::from_utf8_lossy(&added.stdout).into_owned();
    let id: String = said.split('#').nth(1).unwrap_or_else(|| panic!("no id in {said}")).chars().take_while(char::is_ascii_digit).collect();
    let out = rc(dir, &["context", &id]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The files the "also read" line names, in order.
fn also_read(context: &str) -> Vec<String> {
    let Some(at) = context.find("Also read") else { return Vec::new() };
    context[at..].lines().skip(1).take_while(|l| l.starts_with("  ")).map(|l| l.trim().to_owned()).collect()
}

#[test]
fn handoff_also_read_names_the_definition_first_then_the_callers_and_not_the_cards_files() {
    let dir = project();
    write(dir.path(), "src/card.rs", "pub fn change() { crate::brief::brief_of(); }\n");
    write(dir.path(), "src/brief.rs", "pub fn brief_of() -> String { String::new() }\n");
    write(dir.path(), "src/a_caller.rs", "fn caller() { let _ = crate::brief::brief_of(); }\n");
    write(dir.path(), "src/unrelated.rs", "fn brief_of_other() {}\n");
    write(dir.path(), "target/debug/brief.rs", "fn brief_of() {}\n");
    let context = context_of(dir.path(), "Make `brief_of` name the owner; see `cargo test`", "src/card.rs");
    assert_eq!(also_read(&context), vec!["src/brief.rs", "src/a_caller.rs"], "{context}");
}

#[test]
fn handoff_also_read_is_capped_at_eight() {
    let dir = project();
    write(dir.path(), "src/card.rs", "pub fn change() {}\n");
    for n in 0..12 {
        write(dir.path(), &format!("src/user{n:02}.rs"), "fn f() { crate::Ledger::open(); }\n");
    }
    let context = context_of(dir.path(), "Extend `Ledger`", "src/card.rs");
    let named = also_read(&context);
    assert_eq!(named.len(), 8, "{context}");
    assert_eq!(named[0], "src/user00.rs", "{context}");
}

#[test]
fn handoff_also_read_is_absent_when_the_card_names_no_symbol() {
    let dir = project();
    write(dir.path(), "src/card.rs", "pub fn change() {}\n");
    let context = context_of(dir.path(), "Tidy the module; run `cargo test`", "src/card.rs");
    assert!(!context.contains("Also read"), "{context}");
}
