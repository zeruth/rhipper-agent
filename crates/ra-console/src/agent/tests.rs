//! The tests of the agent loop's local work: what a failed check comes to in the notes.

use super::*;

/// A `cargo test` output with two failures among many passing tests, longer than the tail.
fn sample_output() -> String {
    let mut lines = vec!["running 60 tests".to_owned()];
    for i in 0..50 {
        lines.push(format!("test store::tests::a_passing_test_{i} ... ok"));
    }
    lines.push("test store::tests::a_run_records_its_handoff ... FAILED".to_owned());
    lines.push("test agent::tests::a_check_names_its_failures ... FAILED".to_owned());
    lines.push(String::new());
    lines.push("failures:".to_owned());
    lines.push(String::new());
    lines.push("---- store::tests::a_run_records_its_handoff stdout ----".to_owned());
    lines.push("thread 'store::tests::a_run_records_its_handoff' panicked at crates/ra-roadmap/src/store.rs:42:9:".to_owned());
    lines.push("assertion `left == right` failed".to_owned());
    lines.push("  left: 1".to_owned());
    lines.push(" right: 2".to_owned());
    lines.push(String::new());
    lines.push("---- agent::tests::a_check_names_its_failures stdout ----".to_owned());
    lines.push("thread 'agent::tests::a_check_names_its_failures' panicked at crates/ra-console/src/agent.rs:7:5:".to_owned());
    lines.push("the check did not name them".to_owned());
    lines.push(String::new());
    lines.push("failures:".to_owned());
    lines.push("    agent::tests::a_check_names_its_failures".to_owned());
    lines.push("    store::tests::a_run_records_its_handoff".to_owned());
    lines.push(String::new());
    lines.push("test result: FAILED. 58 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out".to_owned());
    lines.join("\n")
}

#[test]
fn the_failed_tests_lead_the_notes_of_a_failed_check() {
    let failure = check_failure(&sample_output());
    assert_eq!(failure.failed, 2);
    let lines: Vec<&str> = failure.text.lines().collect();
    assert_eq!(lines[0], "test store::tests::a_run_records_its_handoff ... FAILED");
    assert_eq!(lines[1], "test agent::tests::a_check_names_its_failures ... FAILED");
    assert!(lines[2].contains("panicked at crates/ra-roadmap/src/store.rs:42:9"));
    assert!(lines[3].contains("panicked at crates/ra-console/src/agent.rs:7:5"));
    assert_eq!(lines[4], "");
    // the tail follows the failures and is no longer than the forty lines kept
    assert_eq!(lines.len(), 5 + TAIL_LINES);
    assert!(failure.text.ends_with("test result: FAILED. 58 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out"));
    assert!(!failure.text.contains("a_passing_test_0 "));
}

#[test]
fn a_routed_worker_takes_the_model_of_the_card_or_the_rules_and_a_pinned_one_keeps_its_own() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(crate::router::ROUTES_FILE),
        "[[route]]\ncomplexity = [1, 2]\nmodel = \"local/qwen3\"\neffort = \"low\"\n",
    )
    .unwrap();
    let mut node = ra_roadmap::markdown::parse("- 2 ‹task› T {c1}\n    objective: a goal\n").unwrap().remove(0);
    let routed = Worker::new("claude-fable-5-1").with_routing(true);
    let picked = routed.for_card(dir.path(), &node).unwrap();
    assert_eq!((picked.model.as_str(), picked.effort), ("local/qwen3", Some(Effort::Low)));
    // the flag's effort stands over the rule's; a card's own model wins over the rule
    let picked = routed.clone().with_effort(Some(Effort::High)).for_card(dir.path(), &node).unwrap();
    assert_eq!(picked.effort, Some(Effort::High));
    node.assignee = Some(ra_roadmap::model::Assignment { model: "z-ai/glm-5.3-flash".into(), role: "implementer".into() });
    assert_eq!(routed.for_card(dir.path(), &node).unwrap().model, "z-ai/glm-5.3-flash");
    // `--model` pins the run: neither the card nor the rules move it
    let pinned = Worker::new("claude-opus-5-5").for_card(dir.path(), &node).unwrap();
    assert_eq!(pinned.model, "claude-opus-5-5");
}

/// A task with a session made an `age` of seconds before `now`, for the card as it stands.
fn with_session(age: u64, now: u64) -> Node {
    let mut node = ra_roadmap::markdown::parse("- 2 ‹task› T\n    objective: a goal\n    files: a.rs\n").unwrap().remove(0);
    node.session = Some("sess-1".into());
    node.session_of = Some(card_fingerprint(&node));
    node.session_at = Some(now - age);
    node
}

#[test]
fn a_session_within_the_day_is_resumed_and_an_older_one_is_not() {
    let now = 1_800_000_000;
    assert_eq!(resume_of(&with_session(SESSION_LIFE - 60, now), now), Ok(Some("sess-1".to_owned())));
    assert_eq!(resume_of(&with_session(SESSION_LIFE + 60, now), now), Err("the session is older than a day".to_owned()));
    let mut changed = with_session(60, now);
    changed.files.push("b.rs".into());
    assert_eq!(resume_of(&changed, now), Err("the card changed since the session".to_owned()));
    let mut bare = with_session(60, now);
    bare.session_of = None;
    assert_eq!(resume_of(&bare, now), Err("the session has no fingerprint".to_owned()));
    bare.session = None;
    assert_eq!(resume_of(&bare, now), Ok(None));
}

#[test]
fn an_output_without_failed_tests_is_its_tail_alone() {
    let failure = check_failure("error[E0425]: cannot find value `x`\n --> src/lib.rs:1:1\nerror: could not compile");
    assert_eq!(failure.failed, 0);
    assert_eq!(failure.text, "error[E0425]: cannot find value `x`\n --> src/lib.rs:1:1\nerror: could not compile");
}

#[test]
fn the_build_advice_tells_the_window_from_rc_from_neither() {
    let files = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    // the window's own code, or the console code it links, waits for the milestone commit
    assert_eq!(
        build_advice(&files(&["apps/rc-gui/src/ui.rs", "crates/ra-roadmap/src/store.rs"])),
        Some("window: restart on the milestone commit")
    );
    assert_eq!(build_advice(&files(&["crates\\ra-console\\src\\agent.rs"])), Some("window: restart on the milestone commit"));
    // rc and the other crates are live for the next run after an install
    assert_eq!(build_advice(&files(&["apps/rc/src/main.rs"])), Some("rc: install any time, no restart"));
    assert_eq!(build_advice(&files(&["crates/ra-roadmap/src/store.rs", "README.md"])), Some("rc: install any time, no restart"));
    // neither says nothing
    assert_eq!(build_advice(&files(&[".claude/skills/rhipper-pair/SKILL.md", "rhipper.md"])), None);
    assert_eq!(build_advice(&[]), None);
}
