//! The tests of the commands: parsing, the lifecycle names and the tree walk.

use super::*;
use ra_roadmap::store::Store;

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
    let (out, _) = run("add task Rhipper parent 1", &store).await.unwrap();
    assert_eq!(out, vec!["#2 Rhipper"]);
    let (out, _) = run("show", &store).await.unwrap();
    assert!(out.iter().any(|l| l.contains("#2 Rhipper")));
}

#[tokio::test]
async fn adds_a_task_under_its_parent() {
    let store = store().await;
    let (out, _) = run("add task Write the decoder parent 1", &store).await.unwrap();
    assert_eq!(out, vec!["#2 Write the decoder"]);
}

#[tokio::test]
async fn a_task_needs_a_parent() {
    let store = store().await;
    let error = run("add task Orphan", &store).await.unwrap_err();
    assert_eq!(format!("{error:#}"), "a node below a project needs parent <id>");
}

#[tokio::test]
async fn a_project_takes_no_parent() {
    let store = store().await;
    let error = run("add project Root parent 7", &store).await.unwrap_err();
    assert_eq!(format!("{error:#}"), "a project has no parent");
}

#[tokio::test]
async fn sets_states() {
    let store = store().await;
    run("add task T parent 1", &store).await.unwrap();
    let (out, _) = run("set 2 in_progress", &store).await.unwrap();
    assert_eq!(out, vec!["#2 is now in_progress"]);
    let error = run("set 2 nonsense", &store).await.unwrap_err();
    assert_eq!(format!("{error:#}"), "that is no state of the lifecycle");
}

#[tokio::test]
async fn completes_names() {
    assert_eq!(complete("se"), ["set"]);
    assert_eq!(complete("q"), ["quit"]);
}

#[tokio::test]
async fn depths_grow_down_the_tree() {
    let store = store().await;
    run("add task T parent 1", &store).await.unwrap();
    let nodes = store.all().await.unwrap();
    assert_eq!(depth(&nodes, &nodes[0]), 0);
    assert_eq!(depth(&nodes, &nodes[1]), 1);
}
