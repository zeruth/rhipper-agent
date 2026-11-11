//! The tests of the markdown format: round trips, the inline fields and the error cases.

use super::*;
use crate::model::{Assignment, Effort, NodeKind, TaskState};

fn node(id: i64, parent: Option<i64>, kind: NodeKind, title: &str) -> Node {
    Node {
        id,
        parent,
        kind,
        title: title.to_owned(),
        objective: None,
        acceptance: None,
        files: Vec::new(),
        session: None,
        session_at: None,
        session_of: None,
        depends_on: Vec::new(),
        state: TaskState::Defined,
        assignee: None,
        complexity: 3,
        effort: Effort::default(),
        estimate: None,
        notes: None,
        assessed: None,
    }
}

#[test]
fn round_trips_the_estimate_line_and_leaves_the_default_out() {
    use crate::model::Estimate;
    let mut sized = node(8, None, NodeKind::Task, "Sized task");
    sized.estimate = Some(Estimate { tokens: 40_000, turns: 12 });
    let routine = node(9, None, NodeKind::Task, "Routine task");
    let text = render(&[sized, routine]);
    assert!(text.contains("    estimate: 40k tokens, 12 turns"), "{text}");
    assert_eq!(text.matches("estimate:").count(), 1, "no estimate writes no line:\n{text}");
    let parsed = parse(&text).unwrap();
    assert_eq!(parsed[0].estimate, Some(Estimate { tokens: 40_000, turns: 12 }));
    assert_eq!(parsed[1].estimate, None);
    assert_eq!(render(&parsed), text);
    // the bare counts read too, and a line without the turns does not
    let short = parse("- 1 ‹task› T\n    estimate: 2500 3").unwrap();
    assert_eq!(short[0].estimate, Some(Estimate { tokens: 2500, turns: 3 }));
    let error = parse("- 1 ‹task› T\n    estimate: 40k").unwrap_err();
    assert!(format!("{error:#}").contains("the tokens then the turns"), "{error:#}");
}

#[test]
fn round_trips_the_effort_line_and_leaves_medium_out() {
    let mut hard = node(8, None, NodeKind::Task, "Hard task");
    hard.effort = Effort::High;
    let routine = node(9, None, NodeKind::Task, "Routine task");
    let nodes = vec![hard, routine];
    let text = render(&nodes);
    assert!(text.contains("    effort: high"), "{text}");
    assert_eq!(text.matches("effort:").count(), 1, "medium writes no line:\n{text}");
    let parsed = parse(&text).unwrap();
    assert_eq!(parsed[0].effort, Effort::High);
    assert_eq!(parsed[1].effort, Effort::Medium);
    assert_eq!(render(&parsed), text);
    let error = parse("- 1 ‹task› T\n    effort: extreme").unwrap_err();
    assert!(format!("{error:#}").contains("low, medium or high"), "{error:#}");
}

#[test]
fn round_trips_the_assessed_line() {
    // roots, so the render and the parse agree on the depth without a parent in the list
    let mut task = node(4, None, NodeKind::Task, "Storage");
    task.assessed = Some("0123456789abcdef".into());
    let mut done = node(5, None, NodeKind::Task, "Done");
    done.assessed = Some("final".into());
    let nodes = vec![task, done];
    let text = render(&nodes);
    assert!(text.contains("    assessed: 0123456789abcdef"), "{text}");
    let parsed = parse(&text).unwrap();
    assert_eq!(parsed[0].assessed.as_deref(), Some("0123456789abcdef"));
    assert_eq!(parsed[1].assessed.as_deref(), Some("final"));
    assert!(parsed[1].is_final());
    assert!(!parsed[0].is_final());
    assert_eq!(render(&parsed), text);
}

#[test]
fn round_trips_a_tree() {
    let nodes = vec![
        node(1, None, NodeKind::Project, "Rhipper Agent"),
        node(2, Some(1), NodeKind::Milestone, "Phase 1"),
        node(4, Some(2), NodeKind::Task, "Storage layer"),
    ];
    let text = render(&nodes);
    let parsed = parse(&text).unwrap();
    assert_eq!(parsed.len(), 3);
    assert_eq!(parsed[0].id, 1);
    assert_eq!(parsed[0].parent, None);
    assert_eq!(parsed[1].parent, Some(1));
    assert_eq!(parsed[2].parent, Some(2));
    assert_eq!(parsed[2].title, "Storage layer");
}

#[test]
fn carries_the_detail_fields() {
    let mut task = node(4, Some(1), NodeKind::Task, "Storage");
    task.objective = Some("hold the tree.".into());
    task.acceptance = Some("a round trip.".into());
    task.notes = Some("done once.".into());
    task.depends_on = vec![2, 7];
    task.state = TaskState::Accepted;
    task.assignee = Some(Assignment { role: "implementer".into(), model: "z-ai/glm-5.3-flash".into() });
    task.complexity = 2;
    let text = render(std::slice::from_ref(&task));
    let parsed = parse(&text).unwrap();
    let back = &parsed[0];
    assert_eq!(back.objective.as_deref(), Some("hold the tree."));
    assert_eq!(back.acceptance.as_deref(), Some("a round trip."));
    assert_eq!(back.notes.as_deref(), Some("done once."));
    assert_eq!(back.depends_on, vec![2, 7]);
    assert_eq!(back.state, TaskState::Accepted);
    assert_eq!(back.assignee.as_ref().map(|a| a.model.as_str()), Some("z-ai/glm-5.3-flash"));
    assert_eq!(back.complexity, 2);
}

#[test]
fn round_trips_the_files_line() {
    let mut task = node(6, None, NodeKind::Task, "Files task");
    task.files = vec!["crates/ra-roadmap/src/model.rs".into(), "crates/ra-console/src/agent.rs".into()];
    let text = render(std::slice::from_ref(&task));
    assert!(text.contains("    files: crates/ra-roadmap/src/model.rs crates/ra-console/src/agent.rs"), "{text}");
    let parsed = parse(&text).unwrap();
    assert_eq!(parsed[0].files, task.files);
    assert_eq!(render(&parsed), text);
}

#[test]
fn round_trips_the_session_line() {
    let mut task = node(7, None, NodeKind::Task, "Session task");
    task.session = Some("3f2a9c1e-0b7d-4e55-9a10-6c2d8e4f1b23".into());
    task.session_of = Some("00ff00ff00ff00ff".into());
    task.session_at = Some(1_700_000_000);
    let text = render(std::slice::from_ref(&task));
    assert!(text.contains("    session: 3f2a9c1e-0b7d-4e55-9a10-6c2d8e4f1b23 00ff00ff00ff00ff 1700000000"), "{text}");
    let parsed = parse(&text).unwrap();
    assert_eq!(parsed[0].session, task.session);
    assert_eq!(parsed[0].session_of, task.session_of);
    assert_eq!(parsed[0].session_at, task.session_at);
    assert_eq!(render(&parsed), text);
}

#[test]
fn a_session_line_without_a_fingerprint_still_reads() {
    let parsed = parse("- 1 ‹task› Old\n    session: sess-1\n").unwrap();
    assert_eq!(parsed[0].session.as_deref(), Some("sess-1"));
    assert_eq!(parsed[0].session_of, None);
    assert_eq!(parsed[0].session_at, None);
}

#[test]
fn round_trips_through_two_passes() {
    let mut nodes = vec![node(1, None, NodeKind::Project, "Rhipper Agent"), node(2, Some(1), NodeKind::Task, "First")];
    nodes[1].state = TaskState::InProgress;
    let text = render(&nodes);
    let once = parse(&text).unwrap();
    let twice = parse(&render(&once)).unwrap();
    assert_eq!(once[1].state, twice[1].state);
    assert_eq!(once[1].parent, twice[1].parent);
}

#[test]
fn an_empty_field_writes_no_line() {
    let nodes = vec![node(1, None, NodeKind::Project, "Rhipper")];
    let text = render(&nodes);
    assert!(!text.contains("objective:"), "no detail line without a value:\n{text}");
}

#[test]
fn a_title_may_hold_brackets() {
    let mut task = node(3, Some(1), NodeKind::Task, "Handle [waits-on] lines");
    task.state = TaskState::Review;
    let parsed = parse(&render(std::slice::from_ref(&task))).unwrap();
    assert_eq!(parsed[0].title, "Handle [waits-on] lines");
    assert_eq!(parsed[0].state, TaskState::Review);
}

#[test]
fn a_multiline_note_survives_the_round_trip() {
    let mut task = node(5, Some(1), NodeKind::Task, "Notes task");
    task.notes = Some("first line\nsecond line with a colon: yes\n\\ and a backslash".into());
    let parsed = parse(&render(std::slice::from_ref(&task))).unwrap();
    assert_eq!(parsed[0].notes.as_deref(), Some("first line\nsecond line with a colon: yes\n\\ and a backslash"));
}

#[test]
fn a_bad_line_names_its_number() {
    let error = parse("- oops").unwrap_err();
    assert!(format!("{error:#}").contains("line 1"), "the error names the line: {error:#}");
    let error = parse("- 1 ‹task› T\n    mystery: x").unwrap_err();
    assert!(format!("{error:#}").contains("unknown field mystery"), "the error names the field: {error:#}");
}
