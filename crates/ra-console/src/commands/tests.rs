//! The tests of the commands: parsing, the lifecycle names and the tree walk.

use super::*;
use ra_roadmap::store::Store;

async fn store() -> Store {
    Store::in_memory().await.unwrap()
}

#[tokio::test]
async fn adds_a_project_and_shows_it() {
    let store = store().await;
    let (out, _) = run("add project Rhipper", &store).await.unwrap();
    assert_eq!(out, vec!["#1 Rhipper"]);
    let (out, _) = run("show", &store).await.unwrap();
    assert_eq!(out.len(), 1);
    assert!(out[0].contains("#1 Rhipper"));
}

#[tokio::test]
async fn adds_a_task_under_its_parent() {
    let store = store().await;
    run("add project Rhipper", &store).await.unwrap();
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
    run("add project Rhipper", &store).await.unwrap();
    let (out, _) = run("set 1 in_progress", &store).await.unwrap();
    assert_eq!(out, vec!["#1 is now in_progress"]);
    let error = run("set 1 nonsense", &store).await.unwrap_err();
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
    run("add project R", &store).await.unwrap();
    run("add task T parent 1", &store).await.unwrap();
    let nodes = store.all().await.unwrap();
    assert_eq!(depth(&nodes, &nodes[0]), 0);
    assert_eq!(depth(&nodes, &nodes[1]), 1);
}
