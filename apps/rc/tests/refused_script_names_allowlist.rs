//! A script the allowlist refuses beside an allowed one is answered with the entry to add.

use ra_console::allow_exe::{allowlist_entry, deny_reason};

#[test]
fn a_refused_script_names_the_file_the_nearest_entry_and_the_line_to_add() {
    let globs = vec!["testdata/rhtest/*.exe".to_owned()];
    let hint = allowlist_entry(&globs, r".\testdata\rhtest\other.cmd /q").unwrap();
    assert!(hint.contains("rhipper.toml"), "{hint}");
    assert!(hint.contains("nearest allowed entry is `testdata/rhtest/*.exe`"), "{hint}");
    assert!(hint.contains(r#"binaries = ["testdata/rhtest/*.exe", "testdata/rhtest/*.cmd"]"#), "{hint}");
}

#[test]
fn an_allowed_script_or_a_bare_command_needs_no_entry() {
    let globs = vec!["testdata/rhtest/*.exe".to_owned()];
    assert_eq!(allowlist_entry(&globs, "testdata/rhtest/a.exe"), None);
    assert_eq!(allowlist_entry(&globs, "cargo test"), None);
}

#[test]
fn a_deny_carries_the_line_and_the_cards_note_keeps_it_once() {
    let dir = tempfile::tempdir().unwrap();
    let worktree = dir.path().join("var/worktrees/41");
    std::fs::create_dir_all(&worktree).unwrap();
    let globs = vec!["testdata/rhtest/*.exe".to_owned()];
    let line = r#"binaries = ["testdata/rhtest/*.exe", "testdata/rhtest/*.cmd"]"#;
    for _ in 0..2 {
        let reason = deny_reason(&worktree, &globs, r".\testdata\rhtest\other.cmd /q", "refused.".to_owned());
        assert!(reason.starts_with("refused. "), "{reason}");
        assert!(reason.contains(line), "{reason}");
    }
    let notes = std::fs::read_to_string(dir.path().join("var/retro.log")).unwrap();
    let mine: Vec<&str> = notes.lines().filter(|l| l.split('\t').nth(1) == Some("41") && l.contains(line)).collect();
    assert_eq!(mine.len(), 1, "{notes}");
}

#[test]
fn a_deny_of_a_bare_command_keeps_its_reason_and_notes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let globs = vec!["testdata/rhtest/*.exe".to_owned()];
    assert_eq!(deny_reason(dir.path(), &globs, "sed -i x", "refused.".to_owned()), "refused.");
    assert!(!dir.path().join("var/retro.log").exists());
}
