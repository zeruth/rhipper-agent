use ra_console::agent::{Worker, card_left, ends_live_run, held_by_person, run, stopped_line};
use ra_roadmap::model::{NodeKind, TaskState};
use ra_roadmap::store::{Card, Store};

/// A roadmap in a temp directory with one card in `state`.
async fn card_in(state: TaskState) -> (tempfile::TempDir, Store, i64) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    let id = store.insert_card(Some(1), NodeKind::Task, "Walk", &Card::new()).await.unwrap();
    store.set_state(id, state).await.unwrap();
    (dir, store, id)
}

#[tokio::test]
async fn a_card_set_blocked_during_its_run_is_read_as_left_and_keeps_its_state() {
    let (_dir, store, id) = card_in(TaskState::InProgress).await;
    assert_eq!(card_left(&store, id).await, None, "a card still in progress is the run's");
    store.set_state(id, TaskState::Blocked).await.unwrap();
    assert_eq!(card_left(&store, id).await, Some(TaskState::Blocked));
    assert_eq!(store.read(id).await.unwrap().state, TaskState::Blocked, "reading it leaves the person's state");
}

#[tokio::test]
async fn a_card_cancelled_during_its_run_has_left_the_roadmap_and_ends_it() {
    let (_dir, store, id) = card_in(TaskState::InProgress).await;
    store.set_state(id, TaskState::Cancelled).await.unwrap();
    assert_eq!(card_left(&store, id).await, Some(TaskState::Cancelled));
}

#[tokio::test]
async fn a_run_of_a_blocked_card_does_not_start() {
    let (_dir, store, id) = card_in(TaskState::Blocked).await;
    let lines = run(&store, id, &Worker::new("claude-haiku-4-5")).await.unwrap();
    assert!(lines[0].starts_with(&format!("skipped #{id} Walk: it is blocked")), "{lines:?}");
    assert_eq!(store.read(id).await.unwrap().state, TaskState::Blocked, "the run does not set it in progress");
}

#[test]
fn a_defined_card_is_not_held_from_its_first_run() {
    assert!(!held_by_person(TaskState::Defined));
    assert!(held_by_person(TaskState::Blocked) && held_by_person(TaskState::Cancelled));
}

#[test]
fn blocked_cancelled_and_defined_end_a_live_run() {
    for state in [TaskState::Blocked, TaskState::Cancelled, TaskState::Defined] {
        assert!(ends_live_run(state), "{state}");
    }
}

#[test]
fn the_states_a_run_leaves_a_card_in_do_not_end_it() {
    for state in [TaskState::InProgress, TaskState::Failed, TaskState::Implemented, TaskState::NeedsRevision, TaskState::Accepted] {
        assert!(!ends_live_run(state), "{state}");
    }
}

#[test]
fn the_feed_says_the_run_was_stopped_and_not_continued() {
    let line = stopped_line(7, "Walk", TaskState::Blocked);
    assert!(line.starts_with("stopped #7 Walk: set blocked while its run was live"), "{line}");
    assert!(line.contains("not continued"), "{line}");
}
