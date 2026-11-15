//! A queued card is read again just before its run starts: one that was blocked, accepted,
//! cancelled or implemented since the queue was read is skipped with a line, and the slot goes to
//! the next ready card.

use ra_console::agent::skipped_now;
use ra_roadmap::history::Source;
use ra_roadmap::model::{NodeKind, TaskState};
use ra_roadmap::store::Store;

async fn card_in(state: TaskState) -> (tempfile::TempDir, Store, i64) {
    let dir = tempfile::tempdir().expect("a tempdir");
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper Agent").await.expect("the store");
    let id = store.insert(Some(1), NodeKind::Task, "Queued", None, None, 3).await.expect("a card");
    if state != TaskState::Defined {
        store.set_state_by(id, state, Source::Run).await.expect("the state");
    }
    (dir, store, id)
}

#[tokio::test]
async fn a_card_blocked_after_it_was_queued_is_skipped() {
    let (_dir, store, id) = card_in(TaskState::Blocked).await;
    assert_eq!(skipped_now(&store, id).await, Some(format!("skipped: #{id} is blocked now")));
}

#[tokio::test]
async fn a_card_accepted_cancelled_or_implemented_is_skipped() {
    for state in [TaskState::Accepted, TaskState::Cancelled, TaskState::Implemented, TaskState::Review] {
        let (_dir, store, id) = card_in(state).await;
        assert_eq!(skipped_now(&store, id).await, Some(format!("skipped: #{id} is {state} now")), "{state}");
    }
}

#[tokio::test]
async fn a_card_still_ready_is_started() {
    for state in [TaskState::Defined, TaskState::NeedsRevision, TaskState::InProgress] {
        let (_dir, store, id) = card_in(state).await;
        assert_eq!(skipped_now(&store, id).await, None, "{state}");
    }
}
