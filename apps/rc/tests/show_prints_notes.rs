//! `rc show <id>` prints the card's notes after its objective, the failure excerpt whole.

use std::path::Path;
use std::process::Command;

fn rc(dir: &Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_rc")).args(args).current_dir(dir).output().expect("rc runs");
    assert!(out.status.success(), "rc {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn show_by_id_prints_the_notes_with_the_failure_excerpt() {
    let dir = tempfile::tempdir().expect("a tempdir");
    rc(dir.path(), &["init", "--empty"]);
    rc(dir.path(), &["add", "project", "Alpha"]);

    let path = dir.path().join("rhipper.md");
    let text = std::fs::read_to_string(&path).expect("the roadmap");
    let mut out = Vec::new();
    for line in text.lines() {
        out.push(line.to_owned());
        if line.contains("Alpha") {
            out.push("    objective: do the thing".to_owned());
            out.push("    notes: failed\\nthread main panicked\\n   3: demo::faulting_function".to_owned());
        }
    }
    std::fs::write(&path, out.join("\n") + "\n").expect("the roadmap is written");

    let shown = rc(dir.path(), &["show"]);
    let line = shown.lines().find(|l| l.contains("Alpha")).expect("the node is listed");
    let id = line[1..].split_whitespace().next().expect("an id").to_owned();

    let shown = rc(dir.path(), &["show", &id]);
    let objective = shown.find("do the thing").expect("the objective prints");
    let notes = shown.find("notes:").expect("the notes print");
    assert!(objective < notes, "{shown}");
    assert!(shown.contains("thread main panicked"), "{shown}");
    assert!(shown.contains("3: demo::faulting_function"), "{shown}");
}
