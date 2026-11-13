//! The tests of the router: which rule a card falls to, and what wins over the rules.

use super::*;

const RULES: &str = "\
[[route]]
complexity = [1, 2]
model = \"local/qwen3\"
effort = \"low\"

[[route]]
complexity = 5
kind = \"task\"
model = \"claude-opus-5-5\"
";

fn card(complexity: u8) -> Node {
    ra_roadmap::markdown::parse(&format!("- 2 ‹task› T {{c{complexity}}}\n    objective: a goal\n")).unwrap().remove(0)
}

#[test]
fn the_worker_table_allows_a_projects_commands_in_both_shells_and_sets_its_environment() {
    let text = "[worker]\nallow = [\"rhtest\", \"Bash(ninja *)\"]\n[worker.env]\nCARGO_BUILD_JOBS = \"6\"\n";
    let router = Router::parse(text).unwrap();
    assert!(router.rules.is_empty());
    assert_eq!(router.worker.rules(), ["Bash(rhtest *)", "Bash(rhtest)", "PowerShell(rhtest *)", "PowerShell(rhtest)", "Bash(ninja *)"]);
    assert_eq!(router.worker.env.get("CARGO_BUILD_JOBS").map(String::as_str), Some("6"));
    assert!(router.worker.allows("rhtest run core"));
    assert!(router.worker.allows("  ninja -C build"));
    assert!(!router.worker.allows("rhtests run"), "a longer word is not the prefix");
    assert!(!router.worker.allows("cargo test"));
    // a file without the table is the empty one
    assert_eq!(Router::parse(RULES).unwrap().worker, WorkerConfig::default());
}

#[test]
fn the_context_cap_is_the_tables_count_and_120k_without_it() {
    assert_eq!(Router::parse("[worker]\ncontext = 80000\n").unwrap().worker.context(), Some(80_000));
    assert_eq!(WorkerConfig::default().context(), None, "no cap unless the project names one");
    assert_eq!(Router::parse("[worker]\ncontext = 0\n").unwrap().worker.context(), None, "zero is no cap");
}

#[test]
fn the_check_timeout_is_the_tables_seconds_and_the_agents_default_without_it() {
    let router = Router::parse("[worker]\ncheck_timeout = 1800\n").unwrap();
    assert_eq!(router.worker.check_timeout(), std::time::Duration::from_secs(1800));
    assert_eq!(WorkerConfig::default().check_timeout(), std::time::Duration::from_secs(1200));
    assert_eq!(
        Router::parse("[worker]\ncheck_timeout = 0\n").unwrap().worker.check_timeout(),
        crate::agent::CHECK_TIMEOUT,
        "zero is no limit of its own"
    );
}

#[test]
fn the_check_is_the_suite_without_one_or_as_a_cargo_test_over_no_package() {
    assert!(WorkerConfig::default().checks_the_suite(), "no check is the workspace tests");
    for (check, suite) in [
        ("cargo test -q", true),
        ("cargo test --workspace --exclude rc", true),
        ("cargo test -p ra-asm -q", false),
        (".\\\\gw.ps1 :core:test", false),
    ] {
        let router = Router::parse(&format!("[worker]\ncheck = \"{check}\"\n")).unwrap();
        assert_eq!(router.worker.checks_the_suite(), suite, "{check}");
    }
}

#[test]
fn two_rules_pick_a_local_model_for_c1_and_the_default_for_c3() {
    let router = Router::parse(RULES).unwrap();
    assert_eq!(router.rules.len(), 2);
    let low = router.choose(&card(1), IMPLEMENTER, "claude-fable-5-1", 0);
    assert_eq!(low, Choice { model: "local/qwen3".into(), effort: Some(Effort::Low), why: Why::Rule(1) });
    let mid = router.choose(&card(3), IMPLEMENTER, "claude-fable-5-1", 0);
    assert_eq!(mid, Choice { model: "claude-fable-5-1".into(), effort: None, why: Why::Default });
    // the second rule is for tasks of c5 only; a reviewer gets no rule at all
    assert_eq!(router.pick(&card(5), IMPLEMENTER, 0).map(|(place, _)| place), Some(2));
    assert_eq!(router.pick(&card(1), "reviewer", 0), None);
}

#[test]
fn a_cards_own_model_wins_over_the_rules() {
    let router = Router::parse(RULES).unwrap();
    let mut node = card(1);
    node.assignee = Some(ra_roadmap::model::Assignment { model: "z-ai/glm-5.3-flash".into(), role: IMPLEMENTER.into() });
    let choice = router.choose(&node, IMPLEMENTER, "claude-fable-5-1", 0);
    assert_eq!(choice, Choice { model: "z-ai/glm-5.3-flash".into(), effort: None, why: Why::Card });
}

#[test]
fn no_file_means_no_rules_and_a_bad_file_is_named() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(Router::load(dir.path()).unwrap(), Router::default());
    std::fs::write(dir.path().join(ROUTES_FILE), "[[route]]\ncomplexity = 1\n").unwrap();
    let error = format!("{:#}", Router::load(dir.path()).unwrap_err());
    assert!(error.contains(ROUTES_FILE) && error.contains("model"), "{error}");
}

/// A c1 task titled `title` with `files`.
fn card_of(title: &str, files: &[&str]) -> Node {
    let mut node = card(1);
    title.clone_into(&mut node.title);
    node.files = files.iter().map(|f| (*f).to_owned()).collect();
    node
}

#[test]
fn a_rule_with_files_fits_only_a_card_touching_them() {
    let router = Router::parse(
        "[[route]]\nfiles = [\"apps/rc-gui/**\", \"crates/ra-roadmap/**\"]\nmodel = \"claude-sonnet-5-5\"\n\n\
         [[route]]\ncomplexity = [1, 2]\nfiles = [\"!crates/**\"]\nmodel = \"claude-haiku-4-5\"\n",
    )
    .unwrap();
    let gui = card_of("T", &["apps/rc-gui/src/main.rs", "crates/ra-console/src/ui.rs"]);
    assert_eq!(router.pick(&gui, IMPLEMENTER, 0).map(|(place, _)| place), Some(1));
    assert_eq!(router.rules[0].matched(&gui), "files apps/rc-gui/**");
    assert!(router.rules[0].fits(&card_of("T", &["crates\\ra-roadmap\\src\\store.rs"]), IMPLEMENTER, 0), "a backslash reads as /");
    assert!(!router.rules[0].fits(&card_of("T", &["apps/rc/src/main.rs"]), IMPLEMENTER, 0));
    assert!(!router.rules[0].fits(&card_of("T", &[]), IMPLEMENTER, 0), "a card with no files touches none");
    // a `!` glob fits a card with no file under it, and none with one
    assert!(router.rules[1].fits(&card_of("T", &["rhipper.toml", "tools/x.py"]), IMPLEMENTER, 0));
    assert!(!router.rules[1].fits(&card_of("T", &["rhipper.toml", "crates/ra-usage/src/lib.rs"]), IMPLEMENTER, 0));
    assert!(glob_matches("crates/*/src/**/*.rs", "crates/ra-tools/src/lib.rs"));
    assert!(!glob_matches("crates/*.rs", "crates/ra-tools/lib.rs"), "`*` stays in its directory");
}

#[test]
fn a_rule_with_words_fits_on_the_title() {
    let router = Router::parse("[[route]]\nwords = [\"gotcha\", \"Docs\"]\nmodel = \"claude-sonnet-5-5\"\n").unwrap();
    let rule = &router.rules[0];
    let gotcha = card_of("A Gotcha for the refused sed", &[]);
    assert!(rule.fits(&gotcha, IMPLEMENTER, 0));
    assert_eq!(rule.matched(&gotcha), "words gotcha");
    assert!(rule.fits(&card_of("The docs of the router", &[]), IMPLEMENTER, 0), "the words are lower-cased too");
    assert!(!rule.fits(&card_of("The router reads the files", &[]), IMPLEMENTER, 0));
    assert_eq!(rule.to_string(), "words gotcha, Docs · implementer → claude-sonnet-5-5");
}

#[test]
fn a_rule_with_rejections_fits_after_the_first_rejection() {
    let router = Router::parse("[[route]]\nrejections = 1\nmodel = \"claude-opus-5-5\"\n").unwrap();
    let node = card(3);
    assert_eq!(router.choose(&node, IMPLEMENTER, "claude-fable-5-1", 0).why, Why::Default);
    let stepped = router.choose(&node, IMPLEMENTER, "claude-fable-5-1", 1);
    assert_eq!((stepped.model.as_str(), &stepped.why), ("claude-opus-5-5", &Why::Rule(1)));
    assert_eq!(router.reason(&node, &stepped), "by rule 1: rejections 1");
    // the count is the reviewer's verdicts that sent the card back, not the failed checks
    let dir = tempfile::tempdir().unwrap();
    let handoff = |verdict: Option<&str>| ra_context::Handoff {
        when: 1,
        model: "claude-fable-5-1".into(),
        outcome: "implemented".into(),
        check: "`cargo test` passed".into(),
        verdict: verdict.map(str::to_owned),
        changed: Vec::new(),
        report: String::new(),
        warnings: Vec::new(),
        read: Vec::new(),
        failures: Vec::new(),
    };
    let mut context = ra_context::TaskContext::open(dir.path(), node.id);
    context.record(handoff(None));
    context.save(dir.path()).unwrap();
    assert_eq!(crate::agent::rejections(dir.path(), node.id), 0);
    context.record(handoff(Some("needs revision (substantive) · 2 point(s) by claude-opus-5-5")));
    context.record(handoff(Some("accepted by claude-opus-5-5")));
    context.save(dir.path()).unwrap();
    assert_eq!(crate::agent::rejections(dir.path(), node.id), 1);
}

#[test]
fn a_rule_reads_as_a_line() {
    let router = Router::parse(RULES).unwrap();
    assert_eq!(router.rules[0].to_string(), "c1..c2 · implementer → local/qwen3 (effort low)");
    assert_eq!(router.rules[1].to_string(), "c5 · task · implementer → claude-opus-5-5");
}
