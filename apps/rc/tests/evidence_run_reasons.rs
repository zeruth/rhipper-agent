//! The multi-run evidence line names, after each run's outcome, why that run ended.

use ra_console::plan::evidence::rerun_line;
use ra_context::Handoff;

fn handoff(outcome: &str, check: &str, verdict: Option<&str>) -> Handoff {
    Handoff {
        when: 1_700_000_000,
        model: "claude-opus-5-5".to_owned(),
        outcome: outcome.to_owned(),
        check: check.to_owned(),
        verdict: verdict.map(str::to_owned),
        changed: vec![],
        report: "done".to_owned(),
        warnings: vec![],
        read: vec![],
        failures: vec![],
    }
}

fn node() -> ra_roadmap::model::Node {
    ra_roadmap::markdown::parse("- 3 ‹task› Twice {c2}\n    objective: a goal\n").unwrap().into_iter().next().expect("a node")
}

#[test]
fn each_run_is_followed_by_the_phrase_for_why_it_ended() {
    let runs = [
        handoff("failed", "cargo test -p x · failed", None),
        handoff("failed", "no check: the turn cap of 40 ended the run before its report", None),
        handoff("failed", "no check: the run failed", None),
        handoff("failed", "merge conflict on main", None),
        handoff("needs revision (trivial)", "cargo test -p x · passed", Some("needs revision (trivial) · 1 point(s) by glm")),
        handoff("implemented", "cargo test -p x · passed", None),
    ];
    let line = rerun_line(&node(), &runs);
    assert!(
        line.text.contains(
            "(failed: check failed, failed: turn cap, failed: worker crashed, failed: merge conflict on main, \
             needs revision (trivial): review sent back, implemented: check passed)"
        ),
        "{}",
        line.text
    );
}
