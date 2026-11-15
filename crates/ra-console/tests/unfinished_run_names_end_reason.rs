use ra_console::plan::evidence::ledger_line_by_run;
use ra_usage::{Call, CardCost, Totals};

fn call(reason: Option<&str>) -> Call {
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
        reason: reason.map(str::to_owned),
    }
}

fn text(reason: Option<&str>) -> String {
    let card = CardCost { node: 1, totals: Totals { calls: 1, turns: 2, prompt_tokens: 800, completion_tokens: 200, ..Totals::default() } };
    ledger_line_by_run(&card, &[call(reason)], &[]).text
}

#[test]
fn an_unreported_run_shows_its_recorded_end_reason() {
    let line = text(Some("worker exit code 1"));
    assert!(line.ends_with("r1 2t 500 unfinished (worker-exit-code-1)"), "{line}");
    assert!(!line.contains("no report"), "{line}");
}

#[test]
fn an_unreported_run_with_no_reason_keeps_the_old_wording() {
    assert!(text(None).ends_with("unfinished (no report)"));
}
