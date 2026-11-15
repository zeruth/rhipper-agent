//! `rc add` refuses a title an accepted or open card already has, ignoring case and the
//! punctuation around it, and names that card.

use std::path::Path;
use std::process::{Command, Output};

fn rc_raw(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rc")).args(args).current_dir(dir).output().expect("rc runs")
}

fn rc(dir: &Path, args: &[&str]) -> String {
    let out = rc_raw(dir, args);
    assert!(out.status.success(), "rc {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn a_title_repeating_an_accepted_or_open_card_is_refused_with_its_number() {
    let dir = tempfile::tempdir().expect("a tempdir");
    rc(dir.path(), &["init", "--empty"]);
    let project = added_id(&rc(dir.path(), &["add", "project", "Alpha"]));
    let parent = added_id(&rc(dir.path(), &["add", "milestone", "Look-back", "--parent", &project]));
    let greeting = added_id(&rc(dir.path(), &["add", "task", "Send the greeting", "--parent", &parent]));
    rc(dir.path(), &["set", &greeting, "accepted"]);

    // accepted: refused whatever the case or the punctuation around the title
    let out = rc_raw(dir.path(), &["add", "task", "  \"send the GREETING.\"", "--parent", &parent]);
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains(&format!("#{greeting} ")), "{said}");

    // open: the same
    let sheep = added_id(&rc(dir.path(), &["add", "task", "Count the sheep", "--parent", &parent]));
    let out = rc_raw(dir.path(), &["add", "task", "COUNT THE SHEEP!", "--parent", &parent]);
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains(&format!("#{sheep} ")), "{said}");

    // a different title is added
    rc(dir.path(), &["add", "task", "Count the goats", "--parent", &parent]);
}

/// The id an `rc add` printed on its first line, `#<id> <title>`.
fn added_id(printed: &str) -> String {
    printed.lines().next().and_then(|l| l.strip_prefix('#')).and_then(|l| l.split_whitespace().next()).expect("an id").to_owned()
}
