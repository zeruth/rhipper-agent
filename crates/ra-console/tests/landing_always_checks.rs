//! A project's `[worker] landing_always` commands join every landing's merged-tree check, after
//! the card's own, whatever crates the card touched.

use ra_console::agent::landing_commands;

fn project(toml: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("rhipper.toml"), toml).unwrap();
    dir
}

#[test]
fn the_always_commands_run_after_the_cards_own() {
    let dir = project("[worker]\nlanding_always = [\"cargo test -q -p data\"]\n");
    let commands = landing_commands(dir.path(), vec!["cargo test -p x".to_owned()], &["docs/a.md".to_owned()]);
    assert_eq!(commands, ["cargo test -p x", "cargo test -q -p data"]);
}

#[test]
fn a_card_without_a_check_still_gets_them() {
    let dir = project("[worker]\nlanding_always = [\"cargo test -q -p data\"]\n");
    assert_eq!(landing_commands(dir.path(), Vec::new(), &[]), ["cargo test -q -p data"]);
}

#[test]
fn a_command_the_card_already_runs_is_not_repeated() {
    let dir = project("[worker]\nlanding_always = [\"cargo test -q -p data\"]\n");
    let commands = landing_commands(dir.path(), vec!["cargo test -q -p data".to_owned()], &[]);
    assert_eq!(commands, ["cargo test -q -p data"]);
}

#[test]
fn a_project_without_them_keeps_its_checks() {
    let dir = project("[worker]\n");
    let commands = landing_commands(dir.path(), vec!["cargo test -p x".to_owned()], &[]);
    assert_eq!(commands, ["cargo test -p x"]);
}
