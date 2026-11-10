//! The tests of the commands: parsing, the lifecycle names and the tree walk.

use super::*;
use ra_roadmap::store::Store;

/// The model of the badge in the tests; no test talks to a network.
const MODEL: &str = "z-ai/glm-5.3-flash";

async fn store() -> Store {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    // the tempdir lives until the store is dropped from the test's future; leak it for the test
    std::mem::forget(dir);
    store
}

#[tokio::test]
async fn adds_a_project_and_shows_it() {
    let store = store().await;
    let (out, _) = run("add task Rhipper parent 1", &store, MODEL).await.unwrap();
    assert_eq!(out, vec!["#2 Rhipper"]);
    let (out, _) = run("show", &store, MODEL).await.unwrap();
    assert!(out.iter().any(|l| l.contains("#2 Rhipper")));
}

#[tokio::test]
async fn adds_a_task_under_its_parent() {
    let store = store().await;
    let (out, _) = run("add task Write the decoder parent 1", &store, MODEL).await.unwrap();
    assert_eq!(out, vec!["#2 Write the decoder"]);
}

#[tokio::test]
async fn a_task_needs_a_parent() {
    let store = store().await;
    let error = run("add task Orphan", &store, MODEL).await.unwrap_err();
    assert_eq!(format!("{error:#}"), "a node below a project needs parent <id>");
}

#[tokio::test]
async fn a_project_takes_no_parent() {
    let store = store().await;
    let error = run("add project Root parent 7", &store, MODEL).await.unwrap_err();
    assert_eq!(format!("{error:#}"), "a project has no parent");
}

#[tokio::test]
async fn sets_states() {
    let store = store().await;
    run("add task T parent 1", &store, MODEL).await.unwrap();
    let (out, _) = run("set 2 in_progress", &store, MODEL).await.unwrap();
    assert_eq!(out, vec!["#2 is now in_progress"]);
    let error = run("set 2 nonsense", &store, MODEL).await.unwrap_err();
    assert_eq!(format!("{error:#}"), "that is no state of the lifecycle");
}

#[tokio::test]
async fn completes_names() {
    assert_eq!(complete("se"), ["set", "seed"]);
    assert_eq!(complete("q"), ["quit"]);
}

#[tokio::test]
async fn depths_grow_down_the_tree() {
    let store = store().await;
    run("add task T parent 1", &store, MODEL).await.unwrap();
    let nodes = store.all().await.unwrap();
    assert_eq!(depth(&nodes, &nodes[0]), 0);
    assert_eq!(depth(&nodes, &nodes[1]), 1);
}

#[test]
fn the_subtree_walks_preorder() {
    use crate::agent::subtree_preorder;
    use ra_roadmap::model::{Node, NodeKind, TaskState};
    let node = |id: i64, parent: Option<i64>| Node {
        id,
        parent,
        kind: NodeKind::Task,
        title: String::new(),
        objective: None,
        acceptance: None,
        depends_on: Vec::new(),
        state: TaskState::Defined,
        assignee: None,
        complexity: 3,
        notes: None,
    };
    let nodes = vec![node(1, None), node(2, Some(1)), node(3, Some(2)), node(4, Some(2)), node(5, Some(1))];
    assert_eq!(subtree_preorder(&nodes, 2), vec![2, 3, 4]);
    assert_eq!(subtree_preorder(&nodes, 1), vec![1, 2, 3, 4, 5]);
    assert_eq!(subtree_preorder(&nodes, 5), vec![5]);
}

#[tokio::test]
async fn seed_fills_an_empty_roadmap_and_refuses_a_second_time() {
    let store = store().await;
    let (out, _) = run("seed", &store, MODEL).await.unwrap();
    assert!(out[0].contains("planted"), "{}", out[0]);
    let error = run("seed", &store, MODEL).await.unwrap_err();
    assert!(format!("{error:#}").contains("already holds nodes"), "{error:#}");
    let (out, _) = run("show", &store, MODEL).await.unwrap();
    assert!(out.iter().any(|l| l.contains("ping")), "the seed holds the ping task: {out:?}");
}

#[tokio::test]
async fn run_refuses_a_project_and_names_a_missing_key() {
    let store = store().await;
    let error = run("run 1", &store, MODEL).await.unwrap_err();
    assert!(format!("{error:#}").contains("is a project"), "{error:#}");
    run("add task T parent 1", &store, MODEL).await.unwrap();
    // this machine may hold a stored key or a real one in the environment; when neither source
    // holds a key, the run must refuse with the key's name. The env lock covers the read and the
    // removal, so a parallel secret test cannot flip the variable in between.
    let guard = crate::secret::ENV_LOCK.lock();
    let key_configured = std::env::var("OPENROUTER_API_KEY").map_or(false, |k| !k.trim().is_empty()) || crate::secret::has_key();
    if !key_configured {
        // SAFETY: the test holds the env lock; no parallel test reads the key
        unsafe { std::env::remove_var("OPENROUTER_API_KEY") };
        let error = run("run 2", &store, MODEL).await.unwrap_err();
        assert!(format!("{error:#}").contains("OPENROUTER_API_KEY") || format!("{error:#}").contains("key"), "{error:#}");
    }
    drop(guard);
}
