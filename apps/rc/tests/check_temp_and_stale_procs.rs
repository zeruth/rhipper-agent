//! `rc check <card>` in a card's worktree runs with `TEMP` and `TMP` set to the card's own
//! directory, made for it outside any git repository and the same one its worker gets, and stops
//! the processes the worktree's earlier runs left running from its target first.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use ra_providers::claude_code::ClaudeCode;
use ra_providers::{CompletionRequest, Message};

/// The card whose worktree the tests stand in.
const CARD: &str = "77";

/// `rc` in `dir`, with `scratch` as the machine's scratch root, or the process's own when none.
fn rc(dir: &Path, scratch: Option<&Path>, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rc"));
    command.args(args).env_remove("CARGO_TARGET_DIR").current_dir(dir);
    if let Some(scratch) = scratch {
        command.env("RHIPPER_SCRATCH", scratch);
    }
    command.output().expect("rc runs")
}

/// A project `proj` under `base` with the worktree of [`CARD`] and a roadmap whose one task's
/// check, a probe in the worktree, prints `TEMP` and `TMP`; the worktree and the task's id.
fn worktree_with_probe(base: &Path, scratch: Option<&Path>) -> (PathBuf, String) {
    let worktree = base.join("proj").join("var").join("worktrees").join(CARD);
    std::fs::create_dir_all(&worktree).expect("the worktree");
    let (script, body) = if cfg!(windows) {
        ("probe.ps1", "Write-Output \"TEMP=$env:TEMP\"\nWrite-Output \"TMP=$env:TMP\"\n")
    } else {
        ("probe.sh", "echo \"TEMP=$TEMP\"\necho \"TMP=$TMP\"\n")
    };
    std::fs::write(worktree.join(script), body).expect("a probe");
    // the roadmap is the project's: a worktree's `rc` reads the one three levels up
    let out = rc(&base.join("proj"), scratch, &["init", "--empty"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let id_of = |args: &[&str]| {
        let out = rc(&worktree, scratch, args);
        assert!(out.status.success(), "rc {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.lines().next().and_then(|l| l.strip_prefix('#')).and_then(|l| l.split_whitespace().next()).expect("an id").to_owned()
    };
    let project = id_of(&["add", "project", "Alpha"]);
    let milestone = id_of(&["add", "milestone", "M", "--parent", &project]);
    let task = id_of(&["add", "task", "T", "--parent", &milestone, "--acceptance", &format!("`{script}`")]);
    (worktree, task)
}

/// The `TEMP` and the `TMP` the check of `task` ran with.
fn checked_temp(worktree: &Path, scratch: Option<&Path>, task: &str) -> (PathBuf, PathBuf) {
    let out = rc(worktree, scratch, &["check", task]);
    assert_eq!(out.status.code(), Some(0), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let log = String::from_utf8_lossy(&out.stdout).lines().last().expect("the log's path").to_owned();
    let kept = std::fs::read_to_string(log).expect("the log");
    let value =
        |key: &str| PathBuf::from(kept.lines().find_map(|l| l.strip_prefix(key)).unwrap_or_else(|| panic!("{key} in {kept}")).trim());
    (value("TEMP="), value("TMP="))
}

fn outside_any_repo(dir: &Path) -> bool {
    dir.ancestors().all(|above| !above.join(".git").exists())
}

#[test]
fn a_cards_check_gets_its_own_temp_under_the_scratch_root_made_for_it() {
    let base = tempfile::tempdir().expect("a tempdir");
    let scratch = tempfile::tempdir().expect("a scratch root");
    let (worktree, task) = worktree_with_probe(base.path(), Some(scratch.path()));
    let (temp, tmp) = checked_temp(&worktree, Some(scratch.path()), &task);
    assert_eq!(temp, tmp);
    // the scratch root is the test's own, wherever its temp is: the card's directory is under it
    assert_eq!(temp, scratch.path().join("tmp").join("proj").join(format!("run-{CARD}")));
    assert!(temp.is_dir(), "{} is made for the check", temp.display());
    assert!(!temp.starts_with(base.path()), "{} is inside the project", temp.display());
}

#[test]
fn a_cards_worker_and_its_check_share_one_temp() {
    let base = tempfile::tempdir().expect("a tempdir");
    let (worktree, task) = worktree_with_probe(base.path(), None);
    let (checked, _) = checked_temp(&worktree, None, &task);
    let request = CompletionRequest {
        model: "claude-opus-5-5".to_owned(),
        messages: vec![Message::user("Reply with the single word: pong.")],
        temperature: None,
        max_tokens: None,
        tools: Vec::new(),
        workspace: Some(worktree.clone()),
        budget_usd: None,
        resume: None,
        max_turns: None,
        max_context: None,
        effort: None,
        read_only: false,
    };
    let env = ClaudeCode::new().environment(&request);
    for key in ["TEMP", "TMP"] {
        let (_, value) = env.iter().find(|(k, _)| k == key).unwrap_or_else(|| panic!("{key} is set: {env:?}"));
        assert_eq!(Path::new(value), checked, "{key} of the worker");
    }
    assert!(checked.is_dir(), "{} exists before the worker starts", checked.display());
    assert!(outside_any_repo(&checked), "{} is in a git repository", checked.display());
}

/// Stands in for a test the card's earlier run left running: the stale test below starts a copy
/// of this binary on it.
#[test]
#[ignore = "run by a_cards_check_stops_the_processes_left_in_its_target, never alone"]
fn a_test_left_running() {
    std::thread::sleep(std::time::Duration::from_secs(120));
}

#[test]
#[cfg(any(windows, target_os = "linux"))]
fn a_cards_check_stops_the_processes_left_in_its_target() {
    let base = tempfile::tempdir().expect("a tempdir");
    let scratch = tempfile::tempdir().expect("a scratch root");
    let (worktree, task) = worktree_with_probe(base.path(), Some(scratch.path()));
    let deps = scratch.path().join("target").join("proj").join(format!("wt-{CARD}")).join("debug").join("deps");
    std::fs::create_dir_all(&deps).expect("the card's target");
    let me = std::env::current_exe().expect("this test binary");
    let stale = deps.join(me.file_name().expect("a name"));
    std::fs::copy(&me, &stale).expect("a stale copy");
    let mut left = Command::new(&stale).args(["--ignored", "--exact", "a_test_left_running"]).spawn().expect("the stale test starts");
    // a binary under another card's target is not this card's to stop
    let other = scratch.path().join("target").join("proj").join(format!("wt-{CARD}0")).join("debug").join("deps");
    std::fs::create_dir_all(&other).expect("another card's target");
    let theirs = other.join(me.file_name().expect("a name"));
    std::fs::copy(&me, &theirs).expect("another card's copy");
    let mut kept = Command::new(&theirs).args(["--ignored", "--exact", "a_test_left_running"]).spawn().expect("the other test starts");
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(left.try_wait().expect("a status").is_none(), "the stale test is running before the check");

    checked_temp(&worktree, Some(scratch.path()), &task);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut ended = left.try_wait().expect("a status");
    while ended.is_none() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(100));
        ended = left.try_wait().expect("a status");
    }
    let other_ran_on = kept.try_wait().expect("a status").is_none();
    let _ = left.kill();
    let _ = kept.kill();
    let _ = (left.wait(), kept.wait());
    assert!(ended.is_some(), "the check stopped the card's stale test");
    assert!(other_ran_on, "the check left another card's test running");
}
