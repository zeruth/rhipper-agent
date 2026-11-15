//! A review that sends a card back with four or more substantive points files one card per
//! point, each with the point as its objective and the files it names; the card waits on them
//! and is not queued again. Three points or fewer rerun the card as before.

use std::sync::Arc;

use ra_console::agent::{Worker, ready_cards, review};
use ra_roadmap::model::{NodeKind, TaskState};
use ra_roadmap::store::Store;

fn says(text: &str) -> ra_providers::Completion {
    let usage =
        ra_providers::Usage { prompt_tokens: 100, completion_tokens: 10, cached_tokens: 0, cost_usd: Some(0.01), cost_computed: false };
    ra_providers::Completion {
        text: text.to_owned(),
        usage,
        turns: 1,
        turn_usage: vec![ra_providers::TurnUsage::of(&usage)],
        tool_calls: Vec::new(),
        session_id: None,
        gotchas: Vec::new(),
        reads: Vec::new(),
        read_in: 100,
        whole_reads: 0,
    }
}

/// A reviewer that always answers with the same text.
struct Reviewer(String);

impl ra_providers::ModelProvider for Reviewer {
    fn complete(&self, _request: ra_providers::CompletionRequest) -> ra_providers::CompletionFuture<'_> {
        let answer = says(&self.0);
        Box::pin(async move { Ok(answer) })
    }
}

/// A project with one implemented card, reviewed with `answer`.
async fn reviewed(answer: &str) -> (tempfile::TempDir, Store, i64) {
    let dir = tempfile::tempdir().expect("a tempdir");
    std::fs::create_dir_all(dir.path().join("src")).expect("a dir");
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn a() {}\n").expect("a file");
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper Agent").await.expect("the store");
    let id = store.insert(Some(1), NodeKind::Task, "Made", None, None, 3).await.expect("a card");
    store.set_state_by(id, TaskState::Implemented, ra_roadmap::history::Source::Run).await.expect("the state");
    let worker = Worker::new("m").with_provider(Arc::new(Reviewer(answer.to_owned())));
    review(&store, id, &worker).await.expect("the review");
    (dir, store, id)
}

const FOUR: &str = "needs_revision\n- the parser drops the last row of src/lib.rs\n- the header is not written\n- the footer repeats itself\n- the count is off by one";

#[tokio::test]
async fn four_points_file_a_card_each_and_the_parent_waits() {
    let (_dir, store, id) = reviewed(FOUR).await;
    let nodes = store.all().await.expect("the nodes");
    let children: Vec<_> = nodes.iter().filter(|n| n.id != id && n.kind == NodeKind::Task).collect();
    assert_eq!(children.len(), 4, "{children:?}");
    let first = children.iter().find(|n| n.objective.as_deref().is_some_and(|o| o.contains("parser drops"))).expect("a card for it");
    assert_eq!(first.files, vec!["src/lib.rs".to_owned()]);
    let parent = nodes.iter().find(|n| n.id == id).expect("the parent");
    assert!(children.iter().all(|c| parent.depends_on.contains(&c.id)), "{:?}", parent.depends_on);
    let roadmap = store.load().await.expect("the roadmap");
    let queue: Vec<i64> = ready_cards(&roadmap.nodes, &roadmap.gambits, false).into_iter().map(|n| n.id).collect();
    assert!(!queue.contains(&id), "{queue:?}");
}

#[tokio::test]
async fn three_points_rerun_the_card() {
    let answer = "needs_revision\n- the parser drops the last row\n- the header is not written\n- the footer repeats itself";
    let (_dir, store, id) = reviewed(answer).await;
    let nodes = store.all().await.expect("the nodes");
    assert_eq!(nodes.iter().filter(|n| n.kind == NodeKind::Task).count(), 1);
    let roadmap = store.load().await.expect("the roadmap");
    let queue: Vec<i64> = ready_cards(&roadmap.nodes, &roadmap.gambits, false).into_iter().map(|n| n.id).collect();
    assert!(queue.contains(&id), "{queue:?}");
}
