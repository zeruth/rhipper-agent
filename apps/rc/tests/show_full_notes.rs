//! `rc show <id>` prints a card's notes whole and `--tail N` prints the last N lines itself;
//! `rc history <id> --full` prints a run's whole report, and a piped `rc show` is refused with the
//! `--tail` form.

use std::path::Path;
use std::process::Command;

use ra_gotchas::{Catalogue, Conditions, Verdict};

fn rc(dir: &Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_rc")).args(args).current_dir(dir).output().expect("rc runs");
    assert!(out.status.success(), "rc {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A project with one card whose notes run to `count` lines; the card's id.
fn project_with_long_notes(dir: &Path, count: usize) -> String {
    rc(dir, &["init", "--empty"]);
    rc(dir, &["add", "project", "Alpha"]);
    let path = dir.join("rhipper.md");
    let text = std::fs::read_to_string(&path).expect("the roadmap");
    let notes = (1..=count).map(|n| format!("note line {n}")).collect::<Vec<_>>().join("\\n");
    let mut out = Vec::new();
    for line in text.lines() {
        out.push(line.to_owned());
        if line.contains("Alpha") {
            out.push(format!("    notes: {notes}"));
        }
    }
    std::fs::write(&path, out.join("\n") + "\n").expect("the roadmap is written");
    let shown = rc(dir, &["show"]);
    let line = shown.lines().find(|l| l.contains("Alpha")).expect("the card is listed");
    line[1..].split_whitespace().next().expect("an id").to_owned()
}

#[test]
fn show_tail_prints_the_last_lines_and_names_the_cut() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let id = project_with_long_notes(dir.path(), 20);
    let shown = rc(dir.path(), &["show", &id, "--tail", "3"]);
    assert!(shown.contains("note line 20"), "{shown}");
    assert!(!shown.contains("note line 1\n"), "{shown}");
    assert!(shown.contains("--tail"), "{shown}");
    assert!(shown.contains(&format!("rc history {id} --full")), "{shown}");
    assert_eq!(shown.lines().count(), 4, "{shown}");
}

#[test]
fn show_prints_long_notes_whole() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let id = project_with_long_notes(dir.path(), 60);
    let shown = rc(dir.path(), &["show", &id]);
    assert!(shown.contains("note line 1\n"), "{shown}");
    assert!(shown.contains("note line 60\n"), "{shown}");
    assert!(!shown.contains("left out"), "{shown}");
}

#[test]
fn history_full_prints_each_runs_whole_report() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let id = project_with_long_notes(dir.path(), 20);
    let report = (1..=30).map(|n| format!("report line {n}")).collect::<Vec<_>>().join("\n");
    ra_context::TaskContext::leave_note(dir.path(), id.parse().expect("an id"), 1, "model", 1_700_000_000, &report).expect("a note");
    let full = rc(dir.path(), &["history", &id, "--full"]);
    assert!(full.contains("report line 1\n"), "{full}");
    assert!(full.contains("report line 30"), "{full}");
}

#[test]
fn a_piped_rc_show_is_told_the_tail_form() {
    for filter in ["tail -n 40", "head -n 40"] {
        let command = format!("rc show 12 | {filter}");
        let Verdict::Deny { id, reason } = Catalogue::seed().command(&command, Conditions::default()) else {
            panic!("{command} is refused with a hint");
        };
        assert_eq!(id, "piped-or-chained", "{command}");
        assert!(reason.contains("rc show <id> --tail N"), "{reason}");
    }
}
