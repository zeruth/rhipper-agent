use ra_console::agent::held_round_hook;
use ra_context::Handoff;

fn handoff(report: &str, changed: &[&str]) -> Handoff {
    Handoff {
        when: 0,
        model: "m".into(),
        outcome: "implemented".into(),
        check: String::new(),
        verdict: None,
        changed: changed.iter().map(|path| (*path).to_owned()).collect(),
        report: report.into(),
        warnings: Vec::new(),
        read: Vec::new(),
        failures: Vec::new(),
    }
}

#[test]
fn a_blocked_report_with_no_change_holds_the_round() {
    let last = handoff("## Blocked: the sdk is not installed\n", &[]);
    let held = held_round_hook(&last, None, "");
    assert_eq!(held.as_deref(), Some("the worker reported it blocked: the sdk is not installed"));
}

#[test]
fn a_blocked_report_that_changed_a_file_leaves_the_round_open() {
    let last = handoff("## Blocked: the sdk is not installed\n", &["a.rs"]);
    assert_eq!(held_round_hook(&last, None, ""), None);
}
