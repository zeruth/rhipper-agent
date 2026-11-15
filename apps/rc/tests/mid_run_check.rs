//! `rc check <card> --mid` runs the card's own check for its worker mid-run, at most twice a
//! run: it prints what the check came to, the first failure and the tail when one fails, notes
//! the use for the handoff and on the activity log, and the hook lets it pass where the
//! project's builds are refused.

use std::path::Path;
use std::process::{Command, Output};

use ra_gotchas::shape::{Shape, Shell};

fn rc(dir: &Path, hub: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rc"))
        .args(args)
        .env_remove("RHIPPER_SCRATCH")
        .env_remove("CARGO_TARGET_DIR")
        .env("RHIPPER_HUB", hub)
        .current_dir(dir)
        .output()
        .expect("rc runs")
}

/// A project with one task for each acceptance; the tasks' ids.
fn project_with(dir: &Path, hub: &Path, acceptances: &[&str]) -> Vec<String> {
    let out = rc(dir, hub, &["init", "--empty"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let id_of = |args: &[&str]| {
        let out = rc(dir, hub, args);
        assert!(out.status.success(), "rc {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.lines().next().and_then(|l| l.strip_prefix('#')).and_then(|l| l.split_whitespace().next()).expect("an id").to_owned()
    };
    let project = id_of(&["add", "project", "Alpha"]);
    let milestone = id_of(&["add", "milestone", "M", "--parent", &project]);
    acceptances.iter().map(|acceptance| id_of(&["add", "task", "T", "--parent", &milestone, "--acceptance", acceptance])).collect()
}

/// A script the check runs, printing `lines` and exiting with `code`.
fn script(dir: &Path, stem: &str, lines: &[&str], code: i32) -> String {
    let (name, body) = if cfg!(windows) {
        let prints: String = lines.iter().map(|line| format!("Write-Output \"{line}\"\n")).collect();
        (format!("{stem}.ps1"), format!("{prints}exit {code}\n"))
    } else {
        let prints: String = lines.iter().map(|line| format!("echo \"{line}\"\n")).collect();
        (format!("{stem}.sh"), format!("{prints}exit {code}\n"))
    };
    std::fs::write(dir.join(&name), body).expect("a script");
    name
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn a_mid_run_check_runs_the_cards_check_twice_and_is_refused_the_third_time() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let hub = tempfile::tempdir().expect("a hub");
    let probe = script(dir.path(), "probe", &["all fine"], 0);
    let ids = project_with(dir.path(), hub.path(), &[&format!("`{probe}`")]);
    let card = &ids[0];
    for _ in 0..2 {
        let out = rc(dir.path(), hub.path(), &["check", card, "--mid"]);
        assert_eq!(out.status.code(), Some(0), "{}{}", stdout(&out), String::from_utf8_lossy(&out.stderr));
        assert!(stdout(&out).contains(&format!("passed {probe}")), "{}", stdout(&out));
    }
    let third = rc(dir.path(), hub.path(), &["check", card, "--mid"]);
    assert_ne!(third.status.code(), Some(0), "{}", stdout(&third));
    let refusal = String::from_utf8_lossy(&third.stderr);
    assert!(refusal.contains("spent its 2 mid-run checks"), "{refusal}");

    let id: i64 = card.parse().expect("an id");
    let lines = ra_console::agent::mid_check_lines(dir.path(), id);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].starts_with("mid-run check 1 of 2: "), "{lines:?}");
    assert!(lines[1].ends_with(" · passed"), "{lines:?}");
    let activity = std::fs::read_to_string(dir.path().join("var").join("activity.log")).expect("the activity log");
    assert!(activity.contains(&format!("mid-run check 2 of 2: {probe} · passed")), "{activity}");

    // a new run of the card has its two again
    ra_console::agent::reset_mid_checks(dir.path(), id);
    assert!(ra_console::agent::mid_check_lines(dir.path(), id).is_empty());
}

#[test]
fn a_failed_mid_run_check_prints_its_first_failure_and_exits_one() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let hub = tempfile::tempdir().expect("a hub");
    let probe = script(dir.path(), "broken", &["test alpha ... FAILED", "the tail of it"], 1);
    let ids = project_with(dir.path(), hub.path(), &[&format!("`{probe}`")]);
    let out = rc(dir.path(), hub.path(), &["check", &ids[0], "--mid"]);
    assert_eq!(out.status.code(), Some(1), "{}{}", stdout(&out), String::from_utf8_lossy(&out.stderr));
    let text = stdout(&out);
    assert!(text.starts_with(&format!("failed {probe}")), "{text}");
    assert!(text.contains("test alpha ... FAILED"), "{text}");
    assert!(text.contains("the tail of it"), "{text}");
    let lines = ra_console::agent::mid_check_lines(dir.path(), ids[0].parse().expect("an id"));
    assert!(lines.iter().all(|line| line.ends_with(" · failed")), "{lines:?}");
}

#[test]
fn the_hook_lets_a_mid_run_check_pass_and_the_prompt_names_it_where_builds_are_refused() {
    let dir = tempfile::tempdir().expect("a tempdir");
    for shell in [Shell::Bash, Shell::PowerShell] {
        assert_eq!(ra_gotchas::shape::judge("rc check 233 --mid", shell, dir.path(), &|_| false), Shape::Allowed);
        assert_ne!(ra_gotchas::shape::judge("rc check 233", shell, dir.path(), &|_| false), Shape::Allowed);
    }
    assert!(ra_gotchas::shape::is_mid_check("rc check --mid 233"));
    assert!(!ra_gotchas::shape::is_mid_check("rc check 233 --mid; cargo build"));
    let block = ra_gotchas::shape::block_with(true, true);
    assert!(block.contains("`rc check <id> --mid`"), "{block}");
    assert!(!ra_gotchas::shape::block_with(true, false).contains("--mid"));
}
