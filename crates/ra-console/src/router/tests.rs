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
fn two_rules_pick_a_local_model_for_c1_and_the_default_for_c3() {
    let router = Router::parse(RULES).unwrap();
    assert_eq!(router.rules.len(), 2);
    let low = router.choose(&card(1), IMPLEMENTER, "claude-fable-5-1");
    assert_eq!(low, Choice { model: "local/qwen3".into(), effort: Some(Effort::Low), why: Why::Rule(1) });
    let mid = router.choose(&card(3), IMPLEMENTER, "claude-fable-5-1");
    assert_eq!(mid, Choice { model: "claude-fable-5-1".into(), effort: None, why: Why::Default });
    // the second rule is for tasks of c5 only; a reviewer gets no rule at all
    assert_eq!(router.pick(&card(5), IMPLEMENTER).map(|(place, _)| place), Some(2));
    assert_eq!(router.pick(&card(1), "reviewer"), None);
}

#[test]
fn a_cards_own_model_wins_over_the_rules() {
    let router = Router::parse(RULES).unwrap();
    let mut node = card(1);
    node.assignee = Some(ra_roadmap::model::Assignment { model: "z-ai/glm-5.3-flash".into(), role: IMPLEMENTER.into() });
    let choice = router.choose(&node, IMPLEMENTER, "claude-fable-5-1");
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

#[test]
fn a_rule_reads_as_a_line() {
    let router = Router::parse(RULES).unwrap();
    assert_eq!(router.rules[0].to_string(), "c1..c2 · implementer → local/qwen3 (effort low)");
    assert_eq!(router.rules[1].to_string(), "c5 · task · implementer → claude-opus-5-5");
}
