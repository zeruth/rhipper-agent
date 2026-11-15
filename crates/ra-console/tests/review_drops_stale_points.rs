use ra_console::agent::{card_print, review_brief};
use ra_console::review::without_stale_points;
use ra_context::{BRIEF_MARK, Handoff, POINTS_MARK, TaskContext};
use ra_roadmap::model::NodeKind;
use ra_roadmap::store::{Card, Edit, Store};

const NOTES: &str = "review by m: needs revision (substantive)\n- 1. crates/ra-asm-lift/src/lib.rs has no Allegrex lift\n- 2. no VFPU\n- 3. no syscalls\n\nthe worker's report";

fn handoff(verdict: String) -> Handoff {
    Handoff {
        when: 0,
        model: "m".into(),
        outcome: "needs revision (substantive)".into(),
        check: String::new(),
        verdict: Some(verdict),
        changed: Vec::new(),
        report: "done".into(),
        warnings: Vec::new(),
        read: Vec::new(),
        failures: Vec::new(),
    }
}

#[test]
fn a_review_of_other_words_loses_its_points_and_a_current_one_keeps_them() {
    let (notes, dropped) = without_stale_points(NOTES, Some("aaaa"), "bbbb");
    assert_eq!(dropped, 3);
    assert_eq!(notes, "\nthe worker's report");
    assert_eq!(without_stale_points(NOTES, Some("aaaa"), "aaaa"), (NOTES.to_owned(), 0));
    assert_eq!(without_stale_points(NOTES, None, "aaaa"), (NOTES.to_owned(), 0), "a verdict without a fingerprint is not stale");
}

#[tokio::test]
async fn the_review_of_an_edited_card_does_not_carry_the_points_of_its_earlier_reviews() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    let card = Card { objective: Some("Lift the Allegrex code"), acceptance: Some("`cargo test`"), ..Card::new() };
    let id = store.insert_card(Some(1), NodeKind::Task, "PRX image", &card).await.unwrap();
    let before = store.read(id).await.unwrap();
    let mut task = TaskContext::open(dir.path(), id);
    task.handoffs
        .push(handoff(format!("needs revision (substantive) · 3 point(s) by m{POINTS_MARK}1234{BRIEF_MARK}{}", card_print(&before))));
    task.save(dir.path()).unwrap();
    store.set_notes(id, NOTES).await.unwrap();

    let node = store.read(id).await.unwrap();
    let brief = review_brief(dir.path(), dir.path(), &node).await;
    assert!(brief.contains("no Allegrex lift"), "{brief}");
    assert!(!brief.contains("Earlier review points dropped"), "{brief}");

    store.edit(id, &Edit { objective: Some("Read the PRX image; the Allegrex lift is the next card"), ..Edit::default() }).await.unwrap();
    let node = store.read(id).await.unwrap();
    let brief = review_brief(dir.path(), dir.path(), &node).await;
    assert!(!brief.contains("no Allegrex lift"), "{brief}");
    assert!(!brief.contains("no VFPU"), "{brief}");
    assert!(brief.contains("3 point(s) of earlier reviews"), "{brief}");
    assert!(brief.contains("the worker's report"), "{brief}");
}
