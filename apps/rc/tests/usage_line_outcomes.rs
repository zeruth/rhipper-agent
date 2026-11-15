//! The costliest-cards evidence line joins each run to its handoff outcome and counts the runs it lists.

use ra_console::plan::evidence::ledger_line_by_run;
use ra_context::Handoff;
use ra_usage::{Call, CardCost, Totals};

fn call(turns: u64, role: Option<&str>) -> Call {
    Call {
        at: "2026-11-14 10:00:00".to_owned(),
        model: "claude-opus-5-5".to_owned(),
        role: role.map(str::to_owned),
        turns,
        prompt_tokens: turns * 1000,
        completion_tokens: 0,
        cached_tokens: 0,
        cost_usd: None,
        cost_computed: false,
        read_in: 0,
        reason: None,
    }
}

fn handoff(outcome: &str, check: &str) -> Handoff {
    Handoff {
        when: 1_700_000_000,
        model: "claude-opus-5-5".to_owned(),
        outcome: outcome.to_owned(),
        check: check.to_owned(),
        verdict: None,
        changed: vec![],
        report: "done".to_owned(),
        warnings: vec![],
        read: vec![],
        failures: vec![],
    }
}

#[test]
fn usage_line_outcomes() {
    // the window holds two worker runs and a review; an older run lies outside it
    let card = CardCost { node: 237, totals: Totals { calls: 3, turns: 13, prompt_tokens: 13_000, ..Totals::default() } };
    let calls = [call(10, None), call(1, Some("review")), call(2, None), call(30, None)];
    let handoffs = [
        handoff("failed", "no check: the turn cap ended the run"),
        handoff("continues", "no check: the turn cap ended the run"),
        handoff("implemented", "cargo test · passed"),
    ];
    let line = ledger_line_by_run(&card, &calls, &handoffs);
    assert!(line.text.contains("over 2 runs"), "{}", line.text);
    assert!(line.text.ends_with("; runs: r1 2t 1k continues (the-turn-cap-ended-the-run), r2 10t 1k implemented"), "{}", line.text);
    assert!(!line.text.contains("no report"), "{}", line.text);
}
