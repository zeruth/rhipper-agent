use ra_console::agent::short_run_note;
use ra_context::Handoff;

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

#[test]
fn a_short_unfinished_run_gives_its_turns_and_stop_reason() {
    let note = short_run_note(&run("continues", "no check: the turn cap ended the run"), 2).unwrap();
    assert!(note.contains("2 turn(s)"));
    assert!(note.contains("the turn cap ended the run"));
}

#[test]
fn a_short_run_with_no_recorded_reason_says_so() {
    let note = short_run_note(&run("failed", "no check"), 1).unwrap();
    assert!(note.contains("no reason recorded"));
}

#[test]
fn a_run_of_three_turns_or_more_adds_no_note() {
    assert!(short_run_note(&run("continues", "no check: capped"), 3).is_none());
    assert!(short_run_note(&run("continues", "no check: capped"), 20).is_none());
}

#[test]
fn a_finished_short_run_adds_no_note() {
    assert!(short_run_note(&run("implemented", "cargo test passed"), 1).is_none());
}
