//! A card whose last run ended `implemented` and has no verdict yet is not picked again: the
//! queue passes it to the next card, with no other card nothing is ready, and the walk says once
//! that it waits on review. A `needs revision` verdict or a `failed` outcome puts it back.

use std::collections::{HashMap, HashSet};

use ra_console::agent::{WAITS_ON_REVIEW, ready_cards, say_review_waits};
use ra_roadmap::history::Source;
use ra_roadmap::model::{NodeKind, TaskState};
use ra_roadmap::store::Store;

async fn project() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().expect("a tempdir");
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper Agent").await.expect("the store");
    (dir, store)
}

/// The ids the walk's queue holds once card `id` is set to `state`, beside a second defined card.
async fn queue_after(state: TaskState) -> (i64, Vec<i64>) {
    let (_dir, store) = project().await;
    let id = store.insert(Some(1), NodeKind::Task, "Made", None, None, 3).await.expect("a card");
    store.insert(Some(1), NodeKind::Task, "Next", None, None, 3).await.expect("a card");
    store.set_state_by(id, TaskState::Implemented, Source::Run).await.expect("the state");
    store.set_state_by(id, state, Source::Run).await.expect("the state");
    let roadmap = store.load().await.expect("the roadmap");
    (id, ready_cards(&roadmap.nodes, &roadmap.gambits, false).into_iter().map(|n| n.id).collect())
}

#[tokio::test]
async fn an_implemented_card_is_passed_over_for_the_next() {
    let (_dir, store) = project().await;
    let first = store.insert(Some(1), NodeKind::Task, "Made", None, None, 3).await.expect("a card");
    let second = store.insert(Some(1), NodeKind::Task, "Next", None, None, 3).await.expect("a card");
    store.set_state_by(first, TaskState::Implemented, Source::Run).await.expect("the state");
    let roadmap = store.load().await.expect("the roadmap");
    let picked = ra_roadmap::tree::next_ready(&roadmap.nodes, &roadmap.gambits).map(|n| n.id);
    assert_eq!(picked, Some(second));
    let queue: Vec<i64> = ready_cards(&roadmap.nodes, &roadmap.gambits, false).into_iter().map(|n| n.id).collect();
    assert_eq!(queue, vec![second]);
}

#[tokio::test]
async fn an_implemented_card_alone_leaves_nothing_ready() {
    let (_dir, store) = project().await;
    let id = store.insert(Some(1), NodeKind::Task, "Made", None, None, 3).await.expect("a card");
    store.set_state_by(id, TaskState::Implemented, Source::Run).await.expect("the state");
    let roadmap = store.load().await.expect("the roadmap");
    assert!(ra_roadmap::tree::next_ready(&roadmap.nodes, &roadmap.gambits).is_none());
    assert!(ready_cards(&roadmap.nodes, &roadmap.gambits, false).is_empty());
    let worker = ra_console::agent::Worker::new("m");
    let lines = ra_console::agent::run_next(&store, &worker).await.expect("the run reads");
    assert_eq!(lines, vec!["no card is ready".to_owned()]);
}

#[tokio::test]
async fn an_implemented_card_is_said_to_wait_on_review_once() {
    let (dir, store) = project().await;
    let id = store.insert(Some(1), NodeKind::Task, "Made", None, None, 3).await.expect("a card");
    store.set_state_by(id, TaskState::Implemented, Source::Run).await.expect("the state");
    let roadmap = store.load().await.expect("the roadmap");
    let mut said = HashMap::new();
    let first = say_review_waits(dir.path(), &roadmap.nodes, &HashSet::new(), &mut said);
    assert_eq!(first, vec![format!("waits #{id} Made · {WAITS_ON_REVIEW}")]);
    assert!(first[0].contains("waits on review"), "{first:?}");
    let again = say_review_waits(dir.path(), &roadmap.nodes, &HashSet::new(), &mut said);
    assert!(again.is_empty(), "told twice: {again:?}");
}

#[tokio::test]
async fn a_card_whose_review_is_in_flight_is_not_told() {
    let (dir, store) = project().await;
    let id = store.insert(Some(1), NodeKind::Task, "Made", None, None, 3).await.expect("a card");
    store.set_state_by(id, TaskState::Implemented, Source::Run).await.expect("the state");
    let roadmap = store.load().await.expect("the roadmap");
    let lines = say_review_waits(dir.path(), &roadmap.nodes, &HashSet::from([id]), &mut HashMap::new());
    assert!(lines.is_empty(), "{lines:?}");
}

#[tokio::test]
async fn a_needs_revision_verdict_puts_the_card_back_on_the_queue() {
    let (id, queue) = queue_after(TaskState::NeedsRevision).await;
    assert!(queue.contains(&id), "{queue:?}");
}

#[tokio::test]
async fn a_failed_outcome_puts_the_card_back_on_the_queue() {
    let (id, queue) = queue_after(TaskState::Failed).await;
    assert!(queue.contains(&id), "{queue:?}");
}
