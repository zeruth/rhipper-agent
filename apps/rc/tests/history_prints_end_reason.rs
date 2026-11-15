//! The ledger stores why each run ended, `rc history <id>` prints it beside the run's turns and
//! tokens, and the next handoff opens with the last run's reason.

use ra_context::{EndReason, Handoff, LAST_ENDED, TaskContext};
use ra_usage::{Entry, UsageStore};

fn run(node: i64, role: Option<&str>, turns: u64, tokens: u64) -> Entry {
    Entry {
        model: "claude-opus-5-5".to_owned(),
        role: role.map(str::to_owned),
        node: Some(node),
        prompt_tokens: tokens,
        completion_tokens: 100,
        cached_tokens: 0,
        cost_usd: None,
        cost_computed: false,
        turns,
        read_in: 0,
    }
}

fn handoff(outcome: &str, check: &str, report: &str) -> Handoff {
    Handoff {
        when: 1_700_000_000,
        model: "claude-opus-5-5".to_owned(),
        outcome: outcome.to_owned(),
        check: check.to_owned(),
        verdict: None,
        changed: vec![],
        report: report.to_owned(),
        warnings: vec![],
        read: vec![],
        failures: vec![],
    }
}

#[tokio::test]
async fn each_run_row_keeps_its_end_reason_and_the_history_line_prints_it() {
    let ledger = UsageStore::in_memory().await.unwrap();
    ledger.record(&run(7, None, 40, 9_900)).await.unwrap();
    ledger.set_reason(7, EndReason::TurnCap.as_str()).await.unwrap();
    ledger.record(&run(7, None, 12, 4_900)).await.unwrap();
    // a reviewer's call after the run takes no reason: the run's own row does
    ledger.record(&run(7, Some("reviewer"), 1, 300)).await.unwrap();
    ledger.set_reason(7, EndReason::Reported.as_str()).await.unwrap();
    // another card's row is not touched
    ledger.record(&run(8, None, 3, 900)).await.unwrap();

    let calls = ledger.calls_of(Some(7)).await.unwrap();
    let lines: Vec<String> = calls.iter().rev().filter(|call| call.role.is_none()).map(ra_usage::Call::run_line).collect();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].ends_with("opus-5-5 · 40 turn(s) · 10000 tokens · ended: turn cap"), "{}", lines[0]);
    assert!(lines[1].ends_with("opus-5-5 · 12 turn(s) · 5000 tokens · ended: reported"), "{}", lines[1]);
    assert_eq!(calls.iter().find(|call| call.role.is_some()).unwrap().reason, None);
    assert_eq!(ledger.calls_of(Some(8)).await.unwrap()[0].reason, None);
}

#[test]
fn every_reason_reads_back_from_its_stored_words() {
    for reason in [EndReason::TurnCap, EndReason::TokenCeiling, EndReason::Crash, EndReason::Refused, EndReason::Reported] {
        assert_eq!(EndReason::parse(reason.as_str()), Some(reason));
    }
    assert_eq!(EndReason::parse("something else"), None);
}

#[test]
fn a_handoff_names_why_its_run_ended() {
    let cases = [
        (handoff("continues", "no check: the turn cap of 25 ended the run; the card continues", ""), EndReason::TurnCap),
        (handoff("continues", "no check: the context cap of 120k tokens (a turn of 130k) ended the run", ""), EndReason::TokenCeiling),
        (handoff("cut off", "no check: the turn cap of 40 ended the run before its report", ""), EndReason::TurnCap),
        (handoff("failed", "no check: the run failed", "stopped over budget: spent $2.0000 of a $1.00 budget"), EndReason::TokenCeiling),
        (handoff("failed", "no check: the run failed", "the harness exited with code 1"), EndReason::Crash),
        (handoff("failed", "cargo test -p x · failed", "## Blocked\nthe hook refused `cargo run`"), EndReason::Refused),
        (handoff("implemented", "cargo test -p x · passed", "changed `src/lib.rs`"), EndReason::Reported),
    ];
    for (run, reason) in cases {
        assert_eq!(run.end_reason(), reason, "{}", run.check);
    }
}

#[test]
fn the_next_handoff_opens_with_the_last_runs_reason() {
    let task = TaskContext {
        id: 7,
        brief: "Task #7".to_owned(),
        outlines: String::new(),
        handoffs: vec![
            handoff("implemented", "cargo test -p x · passed", "done"),
            handoff("continues", "no check: the turn cap of 25 ended the run; the card continues", "half done"),
        ],
        review: None,
        card_print: None,
        check_output: String::new(),
    };
    let prompt = task.prompt();
    let block = prompt.split_once("Task #7\n").map(|(_, rest)| rest).unwrap();
    let first = block.lines().find(|line| !line.is_empty()).unwrap();
    assert_eq!(first, format!("{LAST_ENDED} turn cap."), "{prompt}");
}
