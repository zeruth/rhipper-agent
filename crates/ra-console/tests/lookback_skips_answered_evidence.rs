use ra_console::retro::{self, Landed};

fn landed() -> Vec<Landed> {
    vec![Landed {
        id: 816,
        title: "Tell a worker five turns from the cap to commit and write its report".to_owned(),
        objective: "A worker five turns short of the turn cap is sent one message to commit and write its report.".to_owned(),
        at: 1_794_000_000,
    }]
}

#[test]
fn a_card_a_landed_one_already_meets_is_answered_and_an_unrelated_one_is_not() {
    let landed = landed();
    let same_title = retro::answered_by(&landed, "Tell a worker ten turns from the cap to commit and write its report", None);
    assert_eq!(same_title.map(|card| card.id), Some(816));
    let same_objective = retro::answered_by(
        &landed,
        "Warn near the cap",
        Some("A worker ten turns short of the turn cap is told to commit and write its report."),
    );
    assert_eq!(same_objective.map(|card| card.id), Some(816));
    assert!(retro::answered_by(&landed, "Quote the clippy warnings in the rerun prompt", Some("The rerun shows each warning.")).is_none());
}

#[test]
fn the_brief_lists_each_landed_card_with_its_objective() {
    let lines = retro::landed_lines(&landed());
    assert_eq!(lines.len(), 1);
    assert!(lines[0].starts_with("#816 Tell a worker five turns"), "{lines:?}");
    assert!(lines[0].contains("turn cap"), "{lines:?}");
}

#[test]
fn a_ledger_time_reads_as_unix_seconds() {
    let call = |at: &str| ra_usage::Call {
        at: at.to_owned(),
        model: String::new(),
        role: None,
        turns: 0,
        prompt_tokens: 0,
        completion_tokens: 0,
        cached_tokens: 0,
        cost_usd: None,
        cost_computed: false,
        read_in: 0,
        reason: None,
    };
    assert_eq!(retro::landed_at(&[call("1970-01-02 00:00:01"), call("1970-01-01 00:00:00")]), Some(86_401));
    assert_eq!(retro::landed_at(&[call("2026-11-15 12:00:00")]), Some(1_794_744_000));
    assert_eq!(retro::landed_at(&[]), None);
}
