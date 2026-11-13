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
fn build_allowance_lets_a_named_test_run_through_uncounted() {
    let dir = tempfile::tempdir().unwrap();
    let tree = dir.path();
    let git = std::process::Command::new("git").arg("init").arg("-q").current_dir(tree).status();
    if !git.is_ok_and(|s| std::process::ExitStatus::success(&s)) {
        return;
    }
    let suite = "rc hold -- cargo test -p ra-console -q";
    let named = "rc hold -- cargo test -p ra-console -q a_run_records_its_handoff";
    // an unchanged tree refuses the suite and allows the named test, as often as asked
    assert!(build_allowance(tree, "s1", suite).is_err());
    for _ in 0..5 {
        assert!(build_allowance(tree, "s1", named).is_ok());
    }
    assert!(build_allowance(tree, "s1", "rc hold -- ./gradlew test --tests Foo").is_ok());
    // a changed tree spends the budget on the suite alone
    std::fs::write(tree.join("a.txt"), "x").unwrap();
    for _ in 0..BUILDS_A_SESSION {
        assert!(build_allowance(tree, "s1", suite).is_ok());
    }
    assert!(build_allowance(tree, "s1", suite).is_err());
    assert!(build_allowance(tree, "s1", named).is_ok());
    // a lint fix rerun starts afresh
    reset_builds(tree, "s1");
    assert!(build_allowance(tree, "s1", suite).is_ok());
}

#[test]
fn build_allowance_takes_a_cargo_test_as_named_only_with_a_filter() {
    assert!(names_tests("cargo test -p x -q some_test"));
    assert!(names_tests("rc hold -- cargo test -p x -q some_test 2>&1 | tail -n 80"));
    assert!(!names_tests("rc hold -- cargo test -p x -q 2>&1 | tail -n 80"));
    assert!(!names_tests("cargo test -p x -q > /dev/null && echo done"));
    assert!(names_tests("cargo test -p x -- --nocapture some_test"));
    assert!(!names_tests("cargo test --test it"));
    assert!(!names_tests("cargo test -p x -q"));
    assert!(!names_tests("cargo test --workspace -q -- --nocapture"));
    assert!(!names_tests("cargo build -p x name"));
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
fn the_file_list_widens_from_the_branch_and_drops_the_new_mark_of_what_it_wrote() {
    let owned = |files: &[&str]| files.iter().map(|f| (*f).to_owned()).collect::<Vec<String>>();
    let card = owned(&["src/lib.rs", "+src/card.rs", "+src/later.rs", "crates/a/"]);
    let changed = owned(&["src/card.rs", "src/lib.rs", "crates/a/src/x.rs", "src/extra.rs"]);
    // the card's order holds, the written new file is a plain path, the directory covers its
    // file and the one file the card left out joins at the end
    assert_eq!(file_list(&card, &changed), owned(&["src/lib.rs", "src/card.rs", "+src/later.rs", "crates/a/", "src/extra.rs"]));
    assert_eq!(file_list(&card, &[]), card);
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

#[tokio::test]
async fn lint_scope_keeps_the_warning_on_the_cards_line_and_drops_the_merged_in_one() {
    let clippy = "warning: unused variable: `x`\n --> src/a.rs:11:9\n  |\n11 |     let x = 1;\n  |         ^\n\nwarning: unused variable: `y`\n --> src/a.rs:40:9\n  |\n40 |     let y = 2;\n  |         ^\n\n";
    let diff =
        "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -10,0 +11,2 @@ fn a() {\n+    let x = 1;\n+    let z = 3;\n";
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    let script: ra_tools::Script = Arc::new(move |command: &str| {
        log.lock().unwrap().push(command.to_owned());
        Ok((0, if command.starts_with("git diff") { diff.to_owned() } else { clippy.to_owned() }))
    });
    let toolbox = ra_tools::Toolbox::new(Path::new(".")).unwrap().with_script(script);
    let lint = lint_changed(&toolbox, Path::new("."), &["src/a.rs".to_owned()], Some("cargo clippy -p probe"), "abc123").await;
    // the diff reads the card's base, not HEAD, so the merge's lines are not the card's
    assert_eq!(seen.lock().unwrap()[0], "git diff -U0 abc123 -- src/a.rs");
    assert_eq!(lint.warnings.len(), 1, "{:?}", lint.warnings);
    assert!(lint.warnings[0].contains("`x`") && lint.warnings[0].contains("--> src/a.rs:11:9"), "{:?}", lint.warnings);
}

/// Runs git in `dir` with a throwaway identity and returns its output, trimmed.
fn git_in(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(["-c", "user.name=test", "-c", "user.email=test@example.com", "-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

#[tokio::test]
async fn lint_scope_reads_a_worktree_against_its_merge_base_so_the_catch_up_merge_is_not_the_cards() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    git_in(&project, &["init", "-q", "-b", "main"]);
    std::fs::write(project.join("a.rs"), "fn a() {}\n").unwrap();
    std::fs::write(project.join("b.rs"), "fn b() {}\n").unwrap();
    git_in(&project, &["add", "-A"]);
    git_in(&project, &["commit", "-q", "-m", "start"]);
    let tree = dir.path().join("wt");
    git_in(&project, &["worktree", "add", "-q", "-b", "card/1", tree.to_str().unwrap()]);
    // a checkpoint of the card, then a commit on main the catch-up merge brings in
    std::fs::write(tree.join("a.rs"), "fn a() {}\nfn card() {}\n").unwrap();
    git_in(&tree, &["commit", "-q", "-am", "checkpoint"]);
    std::fs::write(project.join("b.rs"), "fn b() {}\nfn merged() {}\n").unwrap();
    git_in(&project, &["commit", "-q", "-am", "on main"]);
    git_in(&tree, &["merge", "-q", "--no-edit", "main"]);

    assert_eq!(diff_base(&project, &project).await, "HEAD");
    let base = diff_base(&project, &tree).await;
    assert_eq!(base, git_in(&project, &["rev-parse", "main"]));
    let diff = git_in(&tree, &["diff", "-U0", &base]);
    assert!(diff.contains("+fn card() {}"), "{diff}");
    assert!(!diff.contains("merged"), "{diff}");
}

#[test]
fn lint_scope_puts_each_kept_warning_whole_in_the_rerun_prompt() {
    let mut node = ra_roadmap::markdown::parse("- 2 ‹task› T\n    objective: a goal\n").unwrap().remove(0);
    node.state = TaskState::NeedsRevision;
    let dir = tempfile::tempdir().unwrap();
    let mut task = ra_context::TaskContext::open(dir.path(), 2);
    task.brief = "Task #2: T\n".to_owned();
    let warning = "warning: unused variable: `x`\n --> src/a.rs:11:9\n  |\n11 |     let x = 1;\n  |         ^".to_owned();
    task.record(ra_context::Handoff {
        when: 1,
        model: "m".into(),
        outcome: "needs revision".into(),
        check: "check passed: cargo test -q · lint: 1 warning(s)".into(),
        verdict: None,
        changed: vec!["src/a.rs".into()],
        report: "did it".into(),
        warnings: vec![warning.clone()],
        read: vec![],
        failures: vec![],
    });
    let block = rerun_text(&task, &node, &[]).expect("a run is behind the card");
    let prompt = task_text(&task, Some(&block));
    assert!(prompt.contains(&format!("{warning}\n")), "{prompt}");
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

/// One git command in `dir`, which must succeed.
fn git_ok(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git").args(args).current_dir(dir).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A project with one commit and card #2's worktree on `card/2`, with an identity of its own
/// so the orchestrator's commits need none on the machine.
fn project_with_worktree(root: &Path) -> PathBuf {
    git_ok(root, &["init", "-q"]);
    git_ok(root, &["config", "user.name", "test"]);
    git_ok(root, &["config", "user.email", "test@example.com"]);
    std::fs::write(root.join("lib.rs"), "pub struct S {\n    a: u8,\n}\n").unwrap();
    std::fs::write(root.join(".gitignore"), "var/\n").unwrap();
    git_ok(root, &["add", "-A"]);
    git_ok(root, &["commit", "-q", "-m", "base"]);
    let tree = root.join(WORKTREES_DIR).join("2");
    git_ok(root, &["worktree", "add", "-q", "-b", "card/2", tree.to_str().unwrap()]);
    tree
}

/// A worker whose every command answers `code` and `text`, and the commands it was given.
fn scripted(code: i32, text: &'static str) -> (Worker, Arc<Mutex<Vec<String>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let script: ra_tools::Script = Arc::new(move |command: &str| {
        log.lock().push(command.to_owned());
        Ok((code, text.to_owned()))
    });
    (Worker::new("z-ai/glm-5.3-flash").with_script(script), seen)
}

fn card() -> Node {
    ra_roadmap::markdown::parse("- 2 ‹task› T\n    objective: a goal\n    files: lib.rs\n").unwrap().remove(0)
}

#[tokio::test]
async fn a_checkpoint_holding_a_conflict_marker_is_caught_by_the_merge_proof() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let tree = project_with_worktree(root);
    std::fs::write(tree.join("lib.rs"), "pub struct S {\n<<<<<<< HEAD\n    a: u8,\n=======\n    b: u8,\n>>>>>>> main\n}\n").unwrap();
    let (worker, seen) = scripted(0, "ok\n");
    let lock = tokio::sync::Mutex::new(());
    let lines = checkpoint_and_prove(root, &card(), &worker, lock.lock().await).await;
    assert_eq!(lines[0], "  checkpoint: 1 file(s) committed on card/2", "{lines:?}");
    assert!(lines[1].contains("failed, conflict markers in 1 file(s)"), "{lines:?}");
    // the markers are the next run's first job, and no compile was spent on them
    let block = proof_block(root, 2).expect("the failed proof waits for the next run");
    assert!(block.starts_with("FIRST: the checkpoint on this card's branch (card/2)"), "{block}");
    assert!(block.contains("- lib.rs"), "{block}");
    assert!(seen.lock().is_empty(), "{:?}", seen.lock());
    // the lock was given back with the checkpoint
    assert!(lock.try_lock().is_ok());
}

#[tokio::test]
async fn a_merge_whose_compile_fails_reaches_the_next_prompt_by_the_merge_proof() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let tree = project_with_worktree(root);
    // the project moves on; git merges it in cleanly, and the tree no longer builds
    std::fs::write(root.join("more.rs"), "impl S { fn a(&self) {} }\n").unwrap();
    git_ok(root, &["add", "-A"]);
    git_ok(root, &["commit", "-q", "-m", "more"]);
    let node = card();
    let (line, base) = catch_up_worktree(root, &tree, &node).await.expect("the branch was behind");
    assert!(line.contains("merged into card/2, 1 commit(s) behind"), "{line}");
    let base = base.expect("a clean merge is proven");
    let (failing, seen) = scripted(1, "error[E0124]: field `a` is already declared\n");
    let lines = prove_tree(root, &tree, &node, &failing, &base, "the merge of the project's branch").await;
    assert!(lines[0].contains("failed, the compile `cargo test --workspace -q --no-run` failed"), "{lines:?}");
    assert_eq!(*seen.lock(), ["cargo test --workspace -q --no-run"]);
    let block = proof_block(root, 2).expect("the failed compile waits for the next run");
    assert!(block.starts_with("FIRST: the merge of the project's branch on this card's branch (card/2)"), "{block}");
    assert!(block.contains("error[E0124]: field `a` is already declared"), "{block}");
    // a proof that passes clears it: the run after the fix opens on the card again
    let (passing, _) = scripted(0, "ok\n");
    let lines = prove_tree(root, &tree, &node, &passing, &base, "the checkpoint").await;
    assert!(lines[0].contains("is clean and compiles"), "{lines:?}");
    assert_eq!(proof_block(root, 2), None);
}

#[test]
fn the_merge_proof_compiles_a_check_without_running_it() {
    assert_eq!(compile_of("cargo test -p ra-console -q merge_proof"), "cargo test -p ra-console -q merge_proof --no-run");
    assert_eq!(compile_of("cargo test --workspace -- --nocapture"), "cargo test --workspace --no-run");
    assert_eq!(compile_of(r".\gw.ps1 :core:test --tests Foo"), r".\gw.ps1 :core:testClasses");
    assert_eq!(compile_of("./gradlew test"), "./gradlew testClasses");
    assert_eq!(compile_of("python tools/check.py"), "python tools/check.py");
    assert!(has_conflict_marker("a\n>>>>>>> main\n"));
    assert!(!has_conflict_marker("Title\n=======\n<<<<<<<<<< not one\n"));
}

#[tokio::test]
async fn a_landing_clears_the_merge_proof_its_last_run_left() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let path = proof_path(root, 2);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "FIRST: the checkpoint on this card's branch (card/2) failed.\n").unwrap();
    clean_after(root, 2, false).await;
    assert_eq!(proof_block(root, 2), None);
}

#[tokio::test]
async fn a_card_waits_on_prerequisites_until_its_dependency_is_in_its_base() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_in(root, &["init", "-q"]);
    git_in(root, &["commit", "-q", "--allow-empty", "-m", "start"]);
    // #7's work is on its branch, not yet merged into the project's
    git_in(root, &["checkout", "-q", "-b", "card/7"]);
    git_in(root, &["commit", "-q", "--allow-empty", "-m", "Checkpoint: seven\n\nCard: #7"]);
    git_in(root, &["checkout", "-q", "-"]);
    let card = ra_roadmap::markdown::parse("- 9 ‹task› Dependent\n    waits-on: 7\n").unwrap().remove(0);
    let why = prerequisites_missing(root, &card).await.expect("held for #7");
    assert!(why.contains("#7") && why.contains("card/7"), "{why}");
    // once landed and its branch gone, the trailer in HEAD says it is in the base
    git_in(root, &["merge", "-q", "--no-ff", "-m", "Land seven (#7)\n\nCard: #7", "card/7"]);
    git_in(root, &["branch", "-q", "-D", "card/7"]);
    assert_eq!(prerequisites_missing(root, &card).await, None);
    // a landing on another branch only is not in the base
    git_in(root, &["checkout", "-q", "-b", "elsewhere"]);
    git_in(root, &["commit", "-q", "--allow-empty", "-m", "Land eight (#8)\n\nCard: #8"]);
    git_in(root, &["checkout", "-q", "-"]);
    let later = ra_roadmap::markdown::parse("- 10 ‹task› Later\n    waits-on: 8\n").unwrap().remove(0);
    let why = prerequisites_missing(root, &later).await.expect("held for #8");
    assert!(why.contains("#8") && why.contains("not in HEAD"), "{why}");
    // a card git knows nothing of holds nothing
    let old = ra_roadmap::markdown::parse("- 11 ‹task› Old\n    waits-on: 3\n").unwrap().remove(0);
    assert_eq!(prerequisites_missing(root, &old).await, None);
}

#[tokio::test]
async fn a_card_waits_on_prerequisites_while_a_tool_or_a_path_it_needs_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let card = ra_roadmap::markdown::parse("- 4 ‹task› Pack\n    needs: git rhipper-no-such-tool-211\n").unwrap().remove(0);
    let why = prerequisites_missing(root, &card).await.expect("held for the tool");
    assert!(why.contains("`rhipper-no-such-tool-211`") && why.contains("tool"), "{why}");
    let card = ra_roadmap::markdown::parse("- 5 ‹task› Port\n    needs: git third-party/jdk8u\n").unwrap().remove(0);
    let why = prerequisites_missing(root, &card).await.expect("held for the path");
    assert!(why.contains("`third-party/jdk8u`"), "{why}");
    std::fs::create_dir_all(root.join("third-party/jdk8u")).unwrap();
    assert_eq!(prerequisites_missing(root, &card).await, None, "git runs and the path is there");
}

#[tokio::test]
async fn an_acceptance_command_whose_program_is_missing_cannot_run() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let why = unrunnable(root, "rhipper-no-such-tool-211 test").await.expect("no such program");
    assert!(why.contains("not on the path"), "{why}");
    let why = unrunnable(root, "./tools/check.sh").await.expect("no such script");
    assert!(why.contains("not in the project"), "{why}");
    assert_eq!(unrunnable(root, "git status").await, None);
    let nodes = ra_roadmap::markdown::parse("- 1 ‹task› T\n    acceptance: `./tools/check.sh` passes\n").unwrap();
    let faults = unrunnable_checks(root, &nodes).await;
    assert_eq!(faults.len(), 1, "{faults:?}");
    assert!(faults[0].starts_with("#1 T: its check cannot run"), "{faults:?}");
}

#[test]
fn the_read_roots_of_a_project_and_its_temp_reach_the_harness() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    let made = |path: PathBuf| {
        std::fs::create_dir_all(&path).unwrap();
        std::path::absolute(path).unwrap()
    };
    // a Rust and Gradle project with a gitignored source tree, and toolchains of its own
    let jdk8u = made(project.join("third-party").join("jdk8u"));
    std::fs::write(project.join("Cargo.toml"), "").unwrap();
    std::fs::write(project.join("build.gradle"), "").unwrap();
    let registry = made(dir.path().join("cargo").join("registry"));
    let gradle = made(dir.path().join("gradle").join("caches"));
    let java = made(dir.path().join("jdk"));
    let var = |key: &str| {
        let value = match key {
            "CARGO_HOME" => dir.path().join("cargo"),
            "GRADLE_USER_HOME" => dir.path().join("gradle"),
            "JAVA_HOME" => dir.path().join("jdk"),
            // no home: the rustup and Maven caches are not found, and nothing else is guessed
            _ => return None,
        };
        Some(value.into_os_string())
    };
    let worker = crate::router::WorkerConfig { read: vec!["third-party/jdk8u".to_owned(), "no-such-dir".to_owned()], ..Default::default() };
    let roots = read_roots(&project, &worker, var);
    assert_eq!(roots, [jdk8u.clone(), registry.clone(), gradle.clone(), java.clone()]);

    let worktree = made(project.join("var").join("worktrees").join("208"));
    let request = CompletionRequest {
        model: "claude-opus-5-5".to_owned(),
        messages: vec![Message::user("go")],
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
    let harness = harness_of(&project, &worker, var);
    let line = harness.command_line(&request).unwrap();
    let at = line.iter().position(|a| a == "--add-dir").expect("the read roots are added directories");
    let temp = worktree.join("var").join("tmp");
    let shown = |p: &PathBuf| p.display().to_string();
    assert_eq!(line[at + 1..at + 6], [&jdk8u, &registry, &gradle, &java, &temp].map(shown));
    let env = harness.environment(&request);
    for key in ["TMP", "TEMP", "TMPDIR"] {
        assert!(env.iter().any(|(k, v)| k == key && *v == shown(&temp)), "{env:?}");
    }
}

#[test]
fn a_carried_directory_is_linked_into_the_worktree_and_a_carried_file_copied() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    let worktree = dir.path().join("worktree");
    std::fs::create_dir_all(project.join("third-party").join("jdk8u")).unwrap();
    std::fs::create_dir_all(&worktree).unwrap();
    std::fs::write(project.join("local.properties"), "sdk.dir=C:/sdk").unwrap();
    std::fs::write(project.join("third-party").join("jdk8u").join("A.java"), "class A {}").unwrap();
    let entries = ["local.properties".to_owned(), "third-party/jdk8u".to_owned(), "absent".to_owned()];
    carry(&project, &worktree, &entries, 208);

    let file = worktree.join("local.properties");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "sdk.dir=C:/sdk");
    assert!(!std::fs::symlink_metadata(&file).unwrap().file_type().is_symlink(), "a file is a copy");
    let linked = worktree.join("third-party").join("jdk8u");
    assert!(std::fs::symlink_metadata(&linked).unwrap().file_type().is_symlink(), "a directory is a link");
    // a link, not a copy: what the project's tree gains afterwards is there through it
    std::fs::write(project.join("third-party").join("jdk8u").join("B.java"), "class B {}").unwrap();
    assert_eq!(std::fs::read_to_string(linked.join("B.java")).unwrap(), "class B {}");
    assert!(!worktree.join("absent").exists());
}
