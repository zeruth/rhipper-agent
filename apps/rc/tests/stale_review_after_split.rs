//! A review block older than the card's last objective change is shown under a `stale:` line and
//! its points are not counted as open: the verdict that drew it records the words it judged
//! beside its points (`brief <hash>`), and the handoff compares them with the card's now.

use ra_context::{BRIEF_MARK, Handoff, POINTS_MARK, REVIEW_HEADING, STALE_REVIEW, TaskContext, brief_fingerprint, review_of};

const NOTES: &str = "review by m: needs revision (substantive)\n- first point\n- second point\n\nwrote src/a.rs";

/// The context of card 7 as a rerun opens it: its last run judged on `judged`'s words, the
/// review in its notes, and the card reading `now`'s words.
fn rerun(root: &std::path::Path, judged: (&str, &str), now: (&str, &str)) -> TaskContext {
    let mut task = TaskContext::open(root, 7);
    task.brief = format!("Task #7: {}\nObjective: {}\n", now.0, now.1);
    task.record(Handoff {
        when: 100,
        model: "w".into(),
        outcome: "needs revision (substantive)".into(),
        check: "check passed: cargo test -q · exit code 0".into(),
        verdict: Some(format!(
            "needs revision (substantive) · 2 point(s) by m{POINTS_MARK}0123456789abcdef{BRIEF_MARK}{}",
            brief_fingerprint(judged.0, judged.1)
        )),
        changed: vec!["src/a.rs".into()],
        report: "wrote src/a.rs".into(),
        warnings: Vec::new(),
        read: Vec::new(),
        failures: Vec::new(),
    });
    task.save(root).expect("the context is written");
    // the next run opens the file afresh, as the walk does
    let mut task = TaskContext::open(root, 7);
    task.brief = format!("Task #7: {}\nObjective: {}\n", now.0, now.1);
    task.review = review_of(NOTES);
    task.card_print = Some(brief_fingerprint(now.0, now.1));
    task
}

#[test]
fn a_review_before_the_objective_changed_is_marked_stale_and_has_no_open_points() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let task = rerun(dir.path(), ("A thing", "do a and b"), ("A thing", "do a only"));
    let prompt = task.prompt();
    assert!(prompt.contains(&format!("{STALE_REVIEW}\nreview by m: needs revision (substantive)\n- first point")), "{prompt}");
    assert!(!prompt.contains("answer each"), "{prompt}");
    assert_eq!(task.open_points(), 0);
    task.save(dir.path()).expect("the context is written");
    let file = std::fs::read_to_string(TaskContext::path(dir.path(), 7)).expect("the file");
    assert!(file.contains(&format!("\n{REVIEW_HEADING}\n\n{STALE_REVIEW}\nreview by m")), "{file}");
    // a reopened file keeps the one line, and the review stays stale
    let again = TaskContext { card_print: task.card_print.clone(), ..TaskContext::open(dir.path(), 7) };
    let shown = again.review_shown().expect("the review");
    assert_eq!(shown.matches(STALE_REVIEW).count(), 1, "{shown}");
    assert_eq!(again.open_points(), 0);
}

#[test]
fn a_review_after_the_objective_changed_stays_as_it_is() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let task = rerun(dir.path(), ("A thing", "do a only"), ("A thing", "do a only"));
    let prompt = task.prompt();
    assert!(!prompt.contains(STALE_REVIEW), "{prompt}");
    assert!(prompt.contains("answer each:\nreview by m: needs revision (substantive)\n- first point\n- second point"), "{prompt}");
    assert_eq!(task.open_points(), 2);
}

#[test]
fn the_newest_verdict_alone_decides_and_one_without_a_fingerprint_is_not_stale() {
    let dir = tempfile::tempdir().expect("a temp dir");
    // an older verdict judged other words; the newest, which drew the review, recorded none
    let mut task = rerun(dir.path(), ("A thing", "do a and b"), ("A thing", "do a only"));
    task.record(Handoff {
        when: 200,
        model: "w".into(),
        outcome: "needs revision (substantive)".into(),
        check: "check passed: cargo test -q · exit code 0".into(),
        verdict: Some(format!("needs revision (substantive) · 2 point(s) by m{POINTS_MARK}fedcba9876543210")),
        changed: vec!["src/a.rs".into()],
        report: "wrote src/a.rs again".into(),
        warnings: Vec::new(),
        read: Vec::new(),
        failures: Vec::new(),
    });
    assert!(!task.prompt().contains(STALE_REVIEW));
    assert_eq!(task.open_points(), 2);
}

#[test]
fn a_retitled_card_shows_its_review_stale() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let task = rerun(dir.path(), ("A thing", "do a only"), ("The first half of a thing", "do a only"));
    assert!(task.prompt().contains(STALE_REVIEW));
    assert_eq!(task.open_points(), 0);
}
