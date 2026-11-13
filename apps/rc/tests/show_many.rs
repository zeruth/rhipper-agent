//! `rc show` with several ids: one block per id in the order given, an unknown id one line.

use std::path::Path;
use std::process::Command;

fn rc(dir: &Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_rc")).args(args).current_dir(dir).output().expect("rc runs");
    assert!(out.status.success(), "rc {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The id of the top-level node titled `title` in the roadmap `rc show` prints.
fn id_of(dir: &Path, title: &str) -> String {
    let shown = rc(dir, &["show"]);
    let line = shown.lines().find(|l| l.starts_with('#') && l.contains(title)).expect("the node is listed");
    line[1..].split_whitespace().next().expect("an id").to_owned()
}

#[test]
fn several_ids_print_a_block_each_in_order_and_an_unknown_id_is_one_line() {
    let dir = tempfile::tempdir().expect("a tempdir");
    rc(dir.path(), &["init", "--empty"]);
    rc(dir.path(), &["add", "project", "Alpha"]);
    rc(dir.path(), &["add", "project", "Beta"]);
    let alpha = id_of(dir.path(), "Alpha");
    let beta = id_of(dir.path(), "Beta");

    let shown = rc(dir.path(), &["show", &beta, "99999", &alpha]);
    let lines: Vec<&str> = shown.lines().collect();
    assert_eq!(lines.len(), 3, "{shown}");
    assert!(lines[0].starts_with(&format!("#{beta} Beta")), "{shown}");
    assert_eq!(lines[1], "no such node 99999");
    assert!(lines[2].starts_with(&format!("#{alpha} Alpha")), "{shown}");
}
