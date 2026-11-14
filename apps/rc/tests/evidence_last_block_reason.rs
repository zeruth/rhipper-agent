//! The multi-run evidence line quotes the last run's block reason, clipped to 120 characters,
//! in place of the bare `no verdict yet`.

use ra_console::plan::evidence::rerun_line;
use ra_context::Handoff;

fn handoff(outcome: &str, report: &str) -> Handoff {
    Handoff {
        when: 1_700_000_000,
        model: "claude-opus-5-5".to_owned(),
        outcome: outcome.to_owned(),
        check: "cargo test -p x · passed".to_owned(),
        verdict: None,
        changed: vec![],
        report: report.to_owned(),
        warnings: vec![],
        read: vec![],
        failures: vec![],
    }
}

fn node() -> ra_roadmap::model::Node {
    ra_roadmap::markdown::parse("- 3 ‹task› Twice {c2}\n    objective: a goal\n").unwrap().into_iter().next().expect("a node")
}

#[test]
fn the_line_quotes_the_last_block_reason_clipped() {
    let long = "x".repeat(300);
    let runs = [handoff("failed", "done"), handoff("needs revision", &format!("## Blocked\n{long}\n"))];
    let line = rerun_line(&node(), &runs);
    assert!(line.text.ends_with(&format!("no verdict yet, last reason: {}", "x".repeat(120))), "{}", line.text);
}

#[test]
fn a_run_without_a_reason_stays_bare() {
    let runs = [handoff("failed", "done"), handoff("implemented", "done")];
    let line = rerun_line(&node(), &runs);
    assert!(line.text.ends_with("last verdict: no verdict yet"), "{}", line.text);
}
