use ra_console::plan::evidence::ledger_line_by_run;
use ra_context::Handoff;
use ra_usage::{Call, CardCost, Totals};

fn call() -> Call {
    Call {
        at: "2026-11-15 00:00:00".to_owned(),
        model: "m".to_owned(),
        role: None,
        turns: 2,
        prompt_tokens: 800,
        completion_tokens: 200,
        cached_tokens: 0,
        cost_usd: None,
        cost_computed: false,
        read_in: 0,
        reason: None,
    }
}

fn run(outcome: &str, check: &str) -> Handoff {
    Handoff {
        when: 0,
        model: "m".to_owned(),
        outcome: outcome.to_owned(),
        check: check.to_owned(),
        verdict: None,
        changed: Vec::new(),
        report: String::new(),
        warnings: Vec::new(),
        read: Vec::new(),
        failures: Vec::new(),
    }
}

fn text(handoffs: &[Handoff]) -> String {
    let card = CardCost { node: 1, totals: Totals { calls: 1, turns: 2, prompt_tokens: 800, completion_tokens: 200, ..Totals::default() } };
    ledger_line_by_run(&card, &[call()], handoffs).text
}

#[test]
fn a_run_with_no_report_says_so() {
    assert!(text(&[]).ends_with("r1 2t 500 unfinished (no report)"));
}

#[test]
fn a_capped_run_carries_its_stop_reason() {
    let line = text(&[run("continues", "no check: the turn cap ended the run")]);
    assert!(line.ends_with("continues (the-turn-cap-ended-the-run)"), "{line}");
}

#[test]
fn a_failed_run_without_a_reason_says_so() {
    let line = text(&[run("failed", "no check")]);
    assert!(line.ends_with("failed (no-reason-recorded)"), "{line}");
}

#[test]
fn a_finished_run_carries_no_reason() {
    let line = text(&[run("implemented", "cargo test passed")]);
    assert!(line.ends_with("500 implemented"), "{line}");
}
