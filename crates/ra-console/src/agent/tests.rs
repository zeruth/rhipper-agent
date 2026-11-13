//! The tests of the agent loop's local work: what a failed check comes to in the notes, and
//! what the run line says of the turns.

use super::*;

/// A toolbox whose commands answer from `script`, and the commands it saw.
fn scripted_checks(answers: fn(&str, usize) -> (i32, String)) -> (anyhow::Result<ra_tools::Toolbox>, Arc<std::sync::Mutex<Vec<String>>>) {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    let script: ra_tools::Script = Arc::new(move |command: &str| {
        let mut log = log.lock().unwrap();
        log.push(command.to_owned());
        Ok(answers(command, log.len()))
    });
    (ra_tools::Toolbox::new(Path::new(".")).map(|toolbox| toolbox.with_script(script)), seen)
}

const FAILED_ALONE: &str = "running 2 tests\ntest timing::a_check_is_killed ... FAILED\n\nfailures:\n    timing::a_check_is_killed\n\ntest result: FAILED. 1 passed; 1 failed\nexit code 101";

#[tokio::test]
async fn flaky_rerun_passes_a_test_that_passes_alone_and_names_it() {
    let (toolbox, seen) =
        scripted_checks(
            |_, call| if call == 1 { (101, FAILED_ALONE.to_owned()) } else { (0, "test result: ok. 1 passed\nexit code 0".to_owned()) },
        );
    let runs = run_checks(&toolbox, &["cargo test -p ra-console -q".to_owned()], &[]).await;
    assert!(runs[0].passed);
    assert!(runs[0].text.contains("timing::a_check_is_killed"));
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[1], "cargo test -p ra-console -q -- --exact timing::a_check_is_killed");
}

#[tokio::test]
async fn flaky_rerun_fails_the_card_with_the_second_output_when_the_test_fails_again() {
    let (toolbox, seen) = scripted_checks(|_, call| {
        if call == 1 {
            (101, FAILED_ALONE.to_owned())
        } else {
            (101, "test timing::a_check_is_killed ... FAILED\nsecond run\nexit code 101".to_owned())
        }
    });
    let runs = run_checks(&toolbox, &["cargo test -p ra-console".to_owned()], &[]).await;
    assert!(!runs[0].passed);
    assert!(runs[0].text.contains("second run"));
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[test]
fn a_report_commit_section_gives_the_landing_its_message() {
    let report = "Done.\n\n## Commit\nCap a project's Opus slots at its runners\n\nThe priority order decides who goes first,\nthe runners how many.\n\n## Faster next time\n- nothing\n";
    let (subject, body) = commit_section(report).unwrap();
    assert_eq!(subject, "Cap a project's Opus slots at its runners");
    assert_eq!(body, "The priority order decides who goes first,\nthe runners how many.");
    assert_eq!(commit_section("**Commit:**\n`Fix the thing`\n").unwrap().0, "Fix the thing");
    assert!(commit_section("Done. Nothing else.").is_none());
}

#[test]
fn the_end_line_marks_a_computed_cost_as_an_estimate_and_names_the_cached_share() {
    let computed =
        Usage { prompt_tokens: 1_000_000, completion_tokens: 0, cached_tokens: 900_000, cost_usd: Some(4.12), cost_computed: true };
    assert_eq!(end_cost(&computed), "$4.12 est. · 90% cached");
    let reported = Usage { cost_computed: false, ..computed };
    assert_eq!(end_cost(&reported), "$4.1200 · 90% cached");
    let none = Usage { cost_usd: None, cost_computed: false, ..computed };
    assert_eq!(end_cost(&none), "cost unreported · 90% cached");
}

#[test]
fn the_turn_sizes_lead_with_the_average_and_name_the_largest_when_shown() {
    let usage = Usage { prompt_tokens: 30_000, completion_tokens: 600, cached_tokens: 0, cost_usd: None, cost_computed: false };
    let mut session = Session {
        text: String::new(),
        usage,
        calls: 0,
        turns: 3,
        turn_usage: Vec::new(),
        cap: None,
        capped: None,
        reads: Vec::new(),
        biggest: None,
        read_in: 12_500,
        whole_reads: 9,
    };
    // a provider that showed only the sum: the average over the turns
    assert_eq!(turn_sizes(&session), "avg 10k tokens a turn");
    assert_eq!(tokens_line(&session), "in 30000 (0% cached) out 600 · read-in 12k, 9 files whole");
    session.usage.cached_tokens = 27_300;
    assert_eq!(tokens_line(&session), "in 30000 (91% cached) out 600 · read-in 12k, 9 files whole");
    session.turn_usage = vec![
        TurnUsage { input_tokens: 5_000, output_tokens: 200 },
        TurnUsage { input_tokens: 20_000, output_tokens: 200 },
        TurnUsage { input_tokens: 5_000, output_tokens: 200 },
    ];
    assert_eq!(turn_sizes(&session), "avg 10k tokens a turn, largest 20k");
    // no turns at all: the sum stands for one
    session.turns = 0;
    session.turn_usage.clear();
    assert_eq!(turn_sizes(&session), "avg 30k tokens a turn");
}

#[test]
fn the_files_read_again_are_named_most_first_and_three_at_most() {
    let reads: Vec<String> = ["c.rs", "x86.rs", "c.rs", "abi.rs", "once.rs", "c.rs", "x86.rs", "abi.rs", "z.rs", "z.rs"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    assert_eq!(reread(&reads).as_deref(), Some("c.rs ×3, abi.rs ×2, x86.rs ×2"));
    assert_eq!(reread(&["a.rs".to_owned(), "b.rs".to_owned()]), None);
    assert_eq!(reread(&[]), None);
}

/// A `cargo test` output with two failures among many passing tests, longer than the tail, the
/// failures farther from the end than the tail reaches.
fn sample_output() -> String {
    let mut lines = vec!["running 60 tests".to_owned()];
    for i in 0..50 {
        lines.push(format!("test store::tests::a_passing_test_{i} ... ok"));
    }
    lines.push("test store::tests::a_run_records_its_handoff ... FAILED".to_owned());
    lines.push("test agent::tests::a_check_names_its_failures ... FAILED".to_owned());
    for i in 50..100 {
        lines.push(format!("test store::tests::a_passing_test_{i} ... ok"));
    }
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
    // the first failure leads the tail; the second is counted and left to the `failures:` list
    assert_eq!(lines[0], "test store::tests::a_run_records_its_handoff ... FAILED");
    assert_eq!(lines[1], "");
    assert_eq!(lines.len(), 2 + TAIL_LINES);
    assert!(!failure.text.contains("test agent::tests::a_check_names_its_failures ... FAILED"));
    assert!(failure.text.ends_with("test result: FAILED. 58 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out"));
    assert!(!failure.text.contains("a_passing_test_0 "));
    // a short output still leads with its failure, so the notes read the same way every time
    let short = check_failure("running 1 test\ntest probe ... FAILED\n\ntest result: FAILED. 0 passed; 1 failed\nexit code 101");
    assert_eq!(short.failed, 1);
    assert!(short.text.starts_with("test probe ... FAILED\n\nrunning 1 test\n"), "{}", short.text);
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

#[test]
fn the_digest_block_names_the_other_projects_one_line_each_and_is_nothing_without_them() {
    assert_eq!(digest_block(&[]), "");
    let block = digest_block(&["beta (C:/code/beta) · 2 ready".to_owned(), "gamma (C:/code/gamma) · live: walk · pid 7".to_owned()]);
    assert!(block.starts_with("What the other projects on this machine are doing"), "{block}");
    assert_eq!(block.matches("\n- ").count(), 2);
    assert_eq!(clock(19_676 * 86_400 + 10 * 3600 + 4 * 60 + 59), "10:04Z");
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
fn a_rerun_of_a_card_not_absolute_drops_its_session_and_an_absolute_one_keeps_it() {
    let now = 1_800_000_000;
    let node = with_session(60, now);
    assert_eq!(rerun_resume(&node, true, now), Err("a rerun starts a fresh session".to_owned()));
    assert_eq!(rerun_resume(&node, false, now), Ok(Some("sess-1".to_owned())));
    let mut absolute = node;
    absolute.absolute = true;
    assert_eq!(rerun_resume(&absolute, true, now), Ok(Some("sess-1".to_owned())));
}

/// A card sent back with `notes` and the last run's `report` and `failures`, and its prompt
/// on a rerun as the run builds it.
fn rerun_prompt_of(notes: &str, report: &str, failures: Vec<String>, since: &[String]) -> String {
    let mut node = ra_roadmap::markdown::parse("- 2 ‹task› T\n    objective: a goal\n").unwrap().remove(0);
    node.notes = Some(notes.to_owned());
    node.state = TaskState::NeedsRevision;
    let dir = tempfile::tempdir().unwrap();
    let mut task = ra_context::TaskContext::open(dir.path(), 2);
    task.brief = "Task #2: T\n".to_owned();
    task.review = ra_context::review_of(notes);
    task.record(ra_context::Handoff {
        when: 1,
        model: "m".into(),
        outcome: "needs revision".into(),
        check: "check failed: cargo test -q · 2 test(s) failed".into(),
        verdict: Some("needs revision (substantive) · 9 point(s) by r".into()),
        changed: vec!["src/a.rs".into()],
        report: report.to_owned(),
        warnings: vec![],
        read: vec![],
        failures,
    });
    let block = rerun_text(&task, &node, since).expect("a run is behind the card");
    task_text(&task, Some(&block))
}

#[test]
fn a_rerun_prompt_holds_every_review_point_verbatim_and_the_last_report() {
    let points: Vec<String> = (1..=9).map(|i| format!("- point {i}: `src/a.rs` line {i} {}", "is wrong; ".repeat(i))).collect();
    let notes = format!("review by r: needs revision (substantive)\n{}\n\nwrote src/a.rs and the test", points.join("\n"));
    let report = "check failed: cargo test -q\ntest a ... FAILED\nexit code 101\n\nwrote src/a.rs and the test\nverified by reading";
    let prompt = rerun_prompt_of(&notes, report, Vec::new(), &[]);
    for point in &points {
        assert!(prompt.contains(&format!("{point}\n")), "{point} is not in:\n{prompt}");
    }
    assert!(prompt.contains("The last run's report:\nwrote src/a.rs and the test\nverified by reading\n"), "{prompt}");
    assert!(!prompt.contains("Earlier runs of this task"), "{prompt}");
}

#[test]
fn a_rerun_prompt_holds_every_failing_cargo_test_with_its_error_lines() {
    let mut output = String::from("running 40 tests\n");
    for i in 0..40 {
        output.push_str(&format!("test t{i} ... {}\n", if i % 10 == 3 { "FAILED" } else { "ok" }));
    }
    output.push_str("\nfailures:\n\n");
    for i in [3, 13, 23, 33] {
        output.push_str(&format!("---- t{i} stdout ----\nthread 't{i}' panicked at src/a.rs:{i}:5:\nthe value {i} is off\n\n"));
    }
    output.push_str(&"padding\n".repeat(80));
    output.push_str("test result: FAILED. 36 passed; 4 failed\nexit code 101\n");
    let failures = ra_context::failures_of(&output);
    let notes = "check failed: cargo test -q\ntest t3 ... FAILED\n\npadding\nexit code 101\n\nwrote it";
    let prompt = rerun_prompt_of(notes, "wrote it", failures, &["merge main: Fast-forward".to_owned()]);
    for i in [3, 13, 23, 33] {
        assert!(prompt.contains(&format!("t{i}\nthread 't{i}' panicked at src/a.rs:{i}:5:\nthe value {i} is off")), "t{i}:\n{prompt}");
    }
    assert!(prompt.contains("What changed since the last run:\n- merge main: Fast-forward\n"), "{prompt}");
}

#[test]
fn a_rerun_prompt_holds_every_failing_gradle_test_and_marks_those_failing_on_the_base() {
    let output = "> Task :app:test\n\nGreeterTest > greetsByName() FAILED\n    org.opentest4j.AssertionFailedError: expected: <hi> but was: <ho>\n        at app//GreeterTest.greetsByName(GreeterTest.java:12)\n\nClockTest > ticks() FAILED\n    java.lang.IllegalStateException: no clock\n\n2 tests completed, 2 failed\n\n> Task :app:test FAILED\nexit code 1";
    let base = "ClockTest > ticks() FAILED\n    java.lang.IllegalStateException: no clock\nexit code 1";
    let failures = ra_context::marked_on_base(&ra_context::failures_of(output), &ra_context::failures_of(base));
    let prompt = rerun_prompt_of("check failed: gradle test\nexit code 1\n\ndid it", "did it", failures, &[]);
    assert!(
        prompt.contains("GreeterTest > greetsByName()\n    org.opentest4j.AssertionFailedError: expected: <hi> but was: <ho>\n"),
        "{prompt}"
    );
    assert!(!prompt.contains("greetsByName() · also fails on the base"), "{prompt}");
    assert!(prompt.contains("ClockTest > ticks() · also fails on the base\n    java.lang.IllegalStateException: no clock"), "{prompt}");
}

#[test]
fn what_changed_since_the_last_run_is_the_moves_of_the_tree_and_of_the_environment() {
    let reflog = "HEAD@{300} reset: moving to HEAD~1\nHEAD@{200} commit: a change\nHEAD@{150} merge main: Merge made by the 'ort' strategy.\nHEAD@{50} merge old: Fast-forward";
    assert_eq!(reflog_since(reflog, 100), ["merge main: Merge made by the 'ort' strategy.", "reset: moving to HEAD~1"]);
    let sep = if cfg!(windows) { ";" } else { ":" };
    let before = format!("PATH=/a{sep}/b\nallow=rhtest\n");
    let now = format!("PATH=/a{sep}/c\nallow=rhtest\u{1f}ninja\n");
    assert_eq!(environment_moved(&before, &now), ["the PATH gained /c", "the PATH lost /b", "the allowlist gained ninja"]);
    assert!(environment_moved(&now, &now).is_empty());
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

#[test]
fn the_plan_guard_holds_a_run_when_two_roots_together_reach_the_cap_and_names_the_other() {
    let now = unix_now();
    let reading = |name: &str, utilization: f64, seen_at: u64, live: bool| PlanReading {
        name: name.to_owned(),
        five_hour: Some(ra_providers::limits::Window { utilization, resets_at: now + 3600 }),
        seven_day: None,
        seen_at,
        live,
    };
    // alpha alone, at 80% with a run's share to spare: no hold
    let alpha = reading("alpha", 0.8, now - 600, false);
    assert_eq!(plan_hold(&[alpha.clone()], "alpha", now), None);
    // a run live in beta counts for a share the readings do not show yet: together they reach the cap
    let beta_live = PlanReading { name: "beta".to_owned(), five_hour: None, seven_day: None, seen_at: 0, live: true };
    let hold = plan_hold(&[alpha.clone(), beta_live.clone()], "alpha", now).expect("held");
    assert!(hold.starts_with("hold: the plan's five-hour window is at 80% as alpha last saw it, with a run live in beta"), "{hold}");
    // a project's own live run is not another project's
    assert_eq!(plan_hold(&[alpha.clone(), beta_live], "beta", now), None);
    // the latest reading is the account's level, whichever project took it
    let beta = reading("beta", 0.95, now - 60, false);
    let hold = plan_hold(&[alpha.clone(), beta], "alpha", now).expect("held");
    assert!(hold.contains("at 95% as beta last saw it"), "{hold}");
    // a window that reset does not count, and no reading at all is no hold
    let mut gone = reading("beta", 0.95, now - 60, false);
    gone.five_hour = Some(ra_providers::limits::Window { utilization: 0.95, resets_at: now - 1 });
    assert_eq!(plan_hold(&[alpha, gone], "alpha", now), None);
    assert_eq!(plan_hold(&[], "alpha", now), None);
}

#[test]
fn the_window_is_tight_under_a_quarter_left_by_the_latest_reading_of_any_project() {
    let now = unix_now();
    let reading = |name: &str, utilization: f64, seen_at: u64, resets_at: u64| PlanReading {
        name: name.to_owned(),
        five_hour: Some(ra_providers::limits::Window { utilization, resets_at }),
        seven_day: None,
        seen_at,
        live: false,
    };
    // a tenth left is tight; half left is not; a quarter exactly is not under it
    assert!(plan_tight_in(&[reading("alpha", 0.9, now - 60, now + 3600)], now));
    assert!(!plan_tight_in(&[reading("alpha", 0.5, now - 60, now + 3600)], now));
    assert!(!plan_tight_in(&[reading("alpha", 0.75, now - 60, now + 3600)], now));
    // the latest reading decides, whichever project took it
    assert!(plan_tight_in(&[reading("alpha", 0.5, now - 600, now + 3600), reading("beta", 0.9, now - 60, now + 3600)], now));
    assert!(!plan_tight_in(&[reading("alpha", 0.9, now - 600, now + 3600), reading("beta", 0.5, now - 60, now + 3600)], now));
    // a window that reset, or no reading, is room
    assert!(!plan_tight_in(&[reading("alpha", 0.9, now - 60, now - 1)], now));
    assert!(!plan_tight_in(&[], now));
}

#[test]
fn the_doctor_lays_the_checks_out_as_a_table_and_names_the_required_ones_that_failed() {
    let checks = vec![
        Check::new("claude", true, Ok("2.1.0 (Claude Code), logged in through claude.ai".to_owned())),
        Check::new("local server", false, Err("nothing answers at http://127.0.0.1:8080/v1".to_owned())),
        Check::new("roadmap", true, Err("no rhipper.md; `rc init` makes one".to_owned())),
    ];
    let table = Check::table(&checks);
    assert_eq!(table[0], "   check           found");
    assert_eq!(table[1], "✓  claude          2.1.0 (Claude Code), logged in through claude.ai");
    // a failed optional check is a dash, not a cross: it blocks nothing
    assert_eq!(table[2], "–  local server    nothing answers at http://127.0.0.1:8080/v1");
    assert_eq!(table[3], "✗  roadmap         no rhipper.md; `rc init` makes one");
    assert_eq!(Check::blocking(&checks), vec!["roadmap"]);
    assert!(Check::blocking(&checks[..2]).is_empty());
}

#[tokio::test]
async fn the_doctor_reads_the_roadmap_and_opens_the_ledger_of_a_directory() {
    let dir = tempfile::tempdir().unwrap();
    // no roadmap is a failure that names the remedy; a roadmap that parses is counted
    assert!(roadmap_check(dir.path()).unwrap_err().contains("`rc init` makes one"));
    std::fs::write(dir.path().join(crate::setup::ROADMAP_FILE), "- 1 ‹project› P\n  - 2 ‹task› T {c1}\n    objective: a goal\n").unwrap();
    assert!(roadmap_check(dir.path()).unwrap().ends_with("parses, 2 nodes"));
    std::fs::write(dir.path().join(crate::setup::ROADMAP_FILE), "- 1 ‹project› P\nnot a line the roadmap knows\n").unwrap();
    assert!(roadmap_check(dir.path()).unwrap_err().contains("does not parse"));
    // the ledger is made under var/ when there is none, and opens empty
    assert!(ledger_check(dir.path()).await.unwrap().ends_with("opens, 0 calls"));
    assert!(dir.path().join("var").join("usage.db").is_file());
    // the settings line names the sources a worker loads and what the project has for them
    let settings = settings_check(dir.path()).unwrap();
    assert!(settings.starts_with("a worker loads project,local: .claude/settings.json absent"));
}

#[test]
fn a_fingerprint_changes_with_the_command_the_error_the_diff_and_the_base() {
    let print = check_fingerprint("cargo test -q", "error: boom", "diff", "abc\n");
    assert_eq!(print, check_fingerprint("cargo test -q", "error: boom", "diff", "abc"));
    for other in [
        check_fingerprint("cargo test", "error: boom", "diff", "abc"),
        check_fingerprint("cargo test -q", "error: bang", "diff", "abc"),
        check_fingerprint("cargo test -q", "error: boom", "diff2", "abc"),
        check_fingerprint("cargo test -q", "error: boom", "diff", "abd"),
    ] {
        assert_ne!(print, other);
    }
    // the handoff reads it back off its check line, past the fields that follow it
    let mut handoff = ra_context::Handoff {
        when: 0,
        model: "m".to_owned(),
        outcome: "needs revision".to_owned(),
        check: format!("check failed: cargo test -q{}{print} · on estimate · blocked: x", ra_context::FINGERPRINT_MARK),
        verdict: None,
        changed: vec![],
        report: String::new(),
        warnings: vec![],
        read: vec![],
        failures: vec![],
    };
    assert_eq!(handoff.fingerprint(), Some(print.as_str()));
    handoff.check = "check passed: cargo test -q".to_owned();
    assert_eq!(handoff.fingerprint(), None);
}

#[test]
fn a_setup_failure_is_told_from_a_failure_of_the_code_for_the_fingerprint() {
    assert!(is_setup_failure("error: unexpected argument '--frobnicate' found\n\nUsage: cargo test [OPTIONS]\n"));
    assert!(is_setup_failure("'gradle' is not recognized as an internal or external command\n"));
    assert!(is_setup_failure("SDK location not found. Define a valid SDK location with an ANDROID_HOME\n"));
    // once a build ran, the failure is the code's whatever its words
    assert!(!is_setup_failure("   Compiling x v0.1.0\nerror[E0425]: cannot find value `y`: no such file or directory\n"));
    assert!(!is_setup_failure("test probe ... FAILED\nexit code 101\n"));
    assert_eq!(first_error("warning: a\n\nerror: unexpected argument '-x' found\nUsage"), "error: unexpected argument '-x' found");
    assert_eq!(first_error("\nthe probe broke\n"), "the probe broke");
}

/// A project with one card, #2, and a worker whose model says `report` and whose check answers
/// `check` with its exit code.
async fn blockable(dir: &Path, report: &str, check: (i32, &str)) -> (ra_roadmap::store::Store, Worker) {
    use crate::commands::tests::{Scripted, says};
    let store = ra_roadmap::store::Store::open(&dir.join("rhipper.md"), "Rhipper").await.unwrap();
    crate::commands::run("/add task Probe parent 1", &store, &Worker::new("z-ai/glm-5.3-flash")).await.unwrap();
    let (code, output) = (check.0, check.1.to_owned());
    let script: ra_tools::Script = std::sync::Arc::new(move |_: &str| Ok((code, output.clone())));
    let answers = (0..4).map(|_| says(report, vec![])).collect();
    (store, Worker::new("z-ai/glm-5.3-flash").with_provider(Scripted::new(answers)).with_script(script))
}

/// Whether the feed of `dir` carries a line naming #2 blocked for `reason`.
fn fed_blocked(dir: &Path, reason: &str) -> bool {
    crate::pair::recent(dir, 50).iter().any(|line| line.contains("#2 blocked: ") && line.contains(reason))
}

#[tokio::test]
async fn a_repeated_fingerprint_blocks_the_card_on_its_first_sight() {
    use ra_roadmap::model::TaskState;
    let dir = tempfile::tempdir().unwrap();
    let (store, worker) = blockable(dir.path(), "tried again", (1, "the probe broke\nexit code 1\n")).await;
    // the first failure is a revision; the same one on the same tree holds the card
    crate::commands::run("/run 2", &store, &worker).await.unwrap();
    assert_eq!(store.read(2).await.unwrap().state, TaskState::NeedsRevision);
    let (out, _) = crate::commands::run("/run 2", &store, &worker).await.unwrap();
    assert_eq!(store.read(2).await.unwrap().state, TaskState::Blocked, "{out:?}");
    let reason = "the check failed as the last run's did";
    assert!(out.iter().any(|l| l.contains(&format!("#2 blocked: {reason}")) && l.contains("the probe broke")), "{out:?}");
    assert!(fed_blocked(dir.path(), reason), "{:?}", crate::pair::recent(dir.path(), 50));
    let task = ra_context::TaskContext::open(dir.path(), 2);
    assert_eq!(task.handoffs.last().unwrap().outcome, "blocked");
    assert_eq!(task.handoffs[0].fingerprint(), task.handoffs[1].fingerprint());
}

#[tokio::test]
async fn a_setup_failure_blocks_the_card_on_its_first_sight_with_its_fingerprint_line() {
    let dir = tempfile::tempdir().unwrap();
    let rejected = "error: unexpected argument '--frobnicate' found\n\nUsage: cargo test [OPTIONS]\nexit code 1\n";
    let (store, worker) = blockable(dir.path(), "done", (1, rejected)).await;
    let (out, _) = crate::commands::run("/run 2", &store, &worker).await.unwrap();
    assert_eq!(store.read(2).await.unwrap().state, ra_roadmap::model::TaskState::Blocked, "{out:?}");
    let reason = "the check failed before any build";
    assert!(
        out.iter().any(|l| l.contains(&format!("#2 blocked: {reason}")) && l.contains("unexpected argument '--frobnicate'")),
        "{out:?}"
    );
    assert!(fed_blocked(dir.path(), "unexpected argument '--frobnicate'"), "{:?}", crate::pair::recent(dir.path(), 50));
}

#[tokio::test]
async fn a_blocked_report_that_changed_nothing_holds_the_card_on_its_first_sight_without_a_fingerprint() {
    let dir = tempfile::tempdir().unwrap();
    let report = "I could not start.\n\n## Blocked\nno MIPS LLVM on this machine\n";
    let (store, worker) = blockable(dir.path(), report, (0, "ok\nexit code 0\n")).await;
    let (out, _) = crate::commands::run("/run 2", &store, &worker).await.unwrap();
    assert_eq!(store.read(2).await.unwrap().state, ra_roadmap::model::TaskState::Blocked, "{out:?}");
    assert!(out.iter().any(|l| l.contains("#2 blocked: the worker reported it blocked: no MIPS LLVM on this machine")), "{out:?}");
    assert!(fed_blocked(dir.path(), "no MIPS LLVM on this machine"), "{:?}", crate::pair::recent(dir.path(), 50));
    assert_eq!(ra_context::TaskContext::open(dir.path(), 2).handoffs.last().unwrap().fingerprint(), None);
}

/// A PostToolUse event for a Read of `file` in `root`, with the range the worker gave.
fn read_event(root: &Path, file: &str, offset: Option<u64>, limit: Option<u64>) -> serde_json::Value {
    let mut input = serde_json::json!({"file_path": root.join(file).display().to_string()});
    if let Some(offset) = offset {
        input["offset"] = offset.into();
    }
    if let Some(limit) = limit {
        input["limit"] = limit.into();
    }
    serde_json::json!({"hook_event_name": "PostToolUse", "tool_name": "Read", "tool_input": input, "cwd": root.display().to_string()})
}

#[test]
fn a_whole_file_read_of_a_long_file_is_answered_with_the_outline_once_per_run() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    // a 500-line source with three functions, and a card whose brief names the middle one
    let mut source = String::new();
    for (line, name) in [(1, "alpha"), (201, "beta"), (401, "gamma")] {
        while source.lines().count() < line - 1 {
            source.push_str("    let _ = 0;\n");
        }
        source.push_str(&format!("pub fn {name}() {{\n"));
    }
    while source.lines().count() < 500 {
        source.push_str("    let _ = 0;\n");
    }
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/big.rs"), &source).unwrap();
    std::fs::write(root.join("src/small.rs"), "pub fn tiny() {}\n").unwrap();
    std::fs::create_dir_all(root.join(ra_context::CONTEXT_DIR)).unwrap();
    std::fs::write(root.join(ra_context::CONTEXT_DIR).join("7.md"), "# Task #7 context\n\n## Brief\n\nchange `beta` in src/big.rs\n")
        .unwrap();

    // the card is named outright: the tests run under a worker whose RHIPPER_CARD is another
    // card's, and other tests start runs of their own while this one goes
    let hint = hint_of(root, &read_event(root, "src/big.rs", None, None), Some(7)).expect("a whole-file read gets the hint");
    assert!(hint.contains("rc context 7"), "{hint}");
    assert!(hint.contains("beta 201-400"), "{hint}");
    assert!(!hint.contains("alpha"), "only the function the card names: {hint}");
    // the same file again this run: the worker was told
    assert_eq!(hint_of(root, &read_event(root, "src/big.rs", None, None), Some(7)), None);
    // a ranged read, a short file and no card at all need no hint
    begin_run(root, 7);
    assert_eq!(hint_of(root, &read_event(root, "src/big.rs", Some(200), Some(60)), Some(7)), None);
    assert_eq!(hint_of(root, &read_event(root, "src/big.rs", None, Some(60)), Some(7)), None);
    assert_eq!(hint_of(root, &read_event(root, "src/small.rs", None, None), Some(7)), None);
    assert_eq!(hint_of(root, &read_event(root, "src/big.rs", None, None), None), None);
    // a new run forgets what the last one said
    assert!(hint_of(root, &read_event(root, "src/big.rs", None, None), Some(7)).is_some());
    unsafe { std::env::remove_var(CARD_VAR) };
}
