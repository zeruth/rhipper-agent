//! A card whose landing conflicted with main twice is held, and its context lists the commits
//! on main since its base that touched its files.

use ra_context::{CONFLICTS_HELD, Handoff, LANDING_CONFLICT, TaskContext, landing_conflicts, touching_log_args};

const LOG: &str = "a1b2c3d drop the hook's own check\n9f8e7d6 main decides the opposite\n";

#[test]
fn two_landing_conflicts_hold_the_card_and_its_context_lists_the_touching_commits() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let mut task = TaskContext::open(dir.path(), 320);
    let why = "the merge of card/320 conflicts in src/lib.rs; the merge was aborted";

    assert!(!task.record_landing_conflict(Handoff::landing_conflict(1, why, LOG)), "the first conflict reruns the card");
    task.save(dir.path()).expect("saved");

    let mut task = TaskContext::open(dir.path(), 320);
    assert_eq!(landing_conflicts(&task.handoffs), 1);
    assert!(task.record_landing_conflict(Handoff::landing_conflict(2, why, LOG)), "the second conflict holds it");
    assert_eq!(landing_conflicts(&task.handoffs), CONFLICTS_HELD);
    task.save(dir.path()).expect("saved");

    let text = std::fs::read_to_string(TaskContext::path(dir.path(), 320)).expect("the context file");
    assert!(text.contains(LANDING_CONFLICT), "{text}");
    assert!(text.contains("- a1b2c3d drop the hook's own check"), "{text}");
    assert!(text.contains("- 9f8e7d6 main decides the opposite"), "{text}");
    assert!(text.contains("touching its files (2):"), "{text}");
    assert_eq!(landing_conflicts(&TaskContext::open(dir.path(), 320).handoffs), 2);
}

#[test]
fn other_outcomes_do_not_count_toward_the_hold() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let mut task = TaskContext::open(dir.path(), 7);
    task.record(Handoff::cut_off(1, "m", "the turn cap of 40", Vec::new(), ""));
    assert!(!task.record_landing_conflict(Handoff::landing_conflict(2, "a conflict", "")));
    let text = &task.handoffs.last().expect("a handoff").report;
    assert!(text.contains("touching its files: none found"), "{text}");
}

#[test]
fn the_touching_log_names_the_cards_files_without_their_new_mark() {
    let args = touching_log_args("card/320", &["crates/x/src/lib.rs".to_owned(), "+crates/x/tests/t.rs".to_owned()]);
    assert_eq!(args, ["log", "--format=%h %s", "card/320..HEAD", "--", "crates/x/src/lib.rs", "crates/x/tests/t.rs"]);
}
