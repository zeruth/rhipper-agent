//! The costly-card evidence line splits the cost by run: each run's turns, tokens a turn and outcome.

use ra_console::plan::evidence::ledger_line_by_run;
use ra_context::Handoff;
use ra_usage::{Call, CardCost, Totals};

fn call(turns: u64, tokens: u64, role: Option<&str>) -> Call {
    Call {
        at: "2026-11-14 10:00:00".to_owned(),
        model: "claude-opus-5-5".to_owned(),
        role: role.map(str::to_owned),
        turns,
        prompt_tokens: tokens,
        completion_tokens: 0,
        cached_tokens: 0,
        cost_usd: None,
        cost_computed: false,
        read_in: 0,
        reason: None,
    }
}

fn handoff(outcome: &str) -> Handoff {
    Handoff {
        when: 1_700_000_000,
        model: "claude-opus-5-5".to_owned(),
        outcome: outcome.to_owned(),
        check: "cargo test -p x · passed".to_owned(),
        verdict: None,
        changed: vec![],
        report: "done".to_owned(),
        warnings: vec![],
        read: vec![],
        failures: vec![],
    }
}

#[test]
fn the_evidence_line_lists_each_run_with_its_turns_tokens_and_outcome() {
    let card = CardCost { node: 9, totals: Totals { calls: 3, turns: 25, prompt_tokens: 60_000, ..Totals::default() } };
    // the ledger lists the latest first, and a reviewer's call is not a run
    let calls = [call(22, 51_000 * 22, None), call(1, 900, Some("review")), call(3, 900, None)];
    let handoffs = [handoff("failed"), handoff("needs revision (trivial)")];
    let line = ledger_line_by_run(&card, &calls, &handoffs);
    assert!(line.text.contains("25 turn(s)"), "{}", line.text);
    assert!(line.text.ends_with("; runs: r1 3t 300 failed, r2 22t 51k needs-revision"), "{}", line.text);
}

#[test]
fn a_run_without_a_handoff_is_unfinished() {
    let card = CardCost { node: 9, totals: Totals { calls: 1, turns: 2, ..Totals::default() } };
    let line = ledger_line_by_run(&card, &[call(2, 400, None)], &[]);
    assert!(line.text.ends_with("; runs: r1 2t 200 unfinished"), "{}", line.text);
}
