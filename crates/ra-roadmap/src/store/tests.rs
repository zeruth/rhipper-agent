//! The tests of the markdown store: the file round trip and the mutations.

use super::*;
use crate::model::NodeKind;
use tempfile::tempdir;

async fn store() -> (tempfile::TempDir, Store) {
    let dir = tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Test").await.unwrap();
    (dir, store)
}

#[tokio::test]
async fn a_new_store_writes_one_project() {
    let (_dir, store) = store().await;
    let nodes = store.all().await.unwrap();
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].kind, NodeKind::Project);
    assert_eq!(nodes[0].title, "Test");
}

#[tokio::test]
async fn insert_grows_the_ids() {
    let (_dir, store) = store().await;
    let a = store.insert(None, NodeKind::Task, "A", None, None, 2).await;
    // a task needs a parent
    assert!(a.is_err());
    let root = store.all().await.unwrap().remove(0).id;
    let a = store.insert(Some(root), NodeKind::Task, "A", None, None, 2).await.unwrap();
    let b = store.insert(Some(root), NodeKind::Task, "B", None, None, 2).await.unwrap();
    assert_eq!(b, a + 1);
}

#[tokio::test]
async fn insert_keeps_the_file_in_tree_order() {
    let (_dir, store) = store().await;
    let root = store.all().await.unwrap().remove(0).id;
    let a = store.insert(Some(root), NodeKind::Milestone, "A", None, None, 2).await.unwrap();
    let b = store.insert(Some(root), NodeKind::Milestone, "B", None, None, 2).await.unwrap();
    // a late child of the first milestone lands under it, not under the second
    let child = store.insert(Some(a), NodeKind::Task, "A.1", None, None, 2).await.unwrap();
    let nodes = store.all().await.unwrap();
    assert_eq!(nodes.iter().map(|n| n.id).collect::<Vec<_>>(), vec![root, a, child, b]);
    assert_eq!(store.read(child).await.unwrap().parent, Some(a));
}

#[tokio::test]
async fn mutations_persist_through_a_reopen() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("rhipper.md");
    {
        let store = Store::open(&path, "Test").await.unwrap();
        let root = store.all().await.unwrap().remove(0).id;
        let id = store.insert(Some(root), NodeKind::Task, "Task", Some("do it."), None, 3).await.unwrap();
        store.set_state(id, TaskState::InProgress).await.unwrap();
        store.depend_on(id, root).await.unwrap();
        assert_eq!(node_depends(&store, id).await, vec![root]);
    }
    let store = Store::open(&path, "Test").await.unwrap();
    let node = store.read(2).await.unwrap();
    assert_eq!(node.state, TaskState::InProgress);
    assert_eq!(node.objective.as_deref(), Some("do it."));
    assert_eq!(node.depends_on, vec![1]);
}

#[tokio::test]
async fn set_state_and_assign_round_trip() {
    let (_dir, store) = store().await;
    let root = store.all().await.unwrap().remove(0).id;
    let id = store.insert(Some(root), NodeKind::Task, "Task", None, None, 3).await.unwrap();
    store.set_state(id, TaskState::Review).await.unwrap();
    store.assign(id, Some(&Assignment { role: "reviewer".into(), model: "m".into() })).await.unwrap();
    let node = store.read(id).await.unwrap();
    assert_eq!(node.state, TaskState::Review);
    assert_eq!(node.assignee.as_ref().unwrap().role, "reviewer");
    store.assign(id, None).await.unwrap();
    assert!(store.read(id).await.unwrap().assignee.is_none());
}

#[tokio::test]
async fn set_assessed_marks_and_clears() {
    let (_dir, store) = store().await;
    let root = store.all().await.unwrap().remove(0).id;
    let id = store.insert(Some(root), NodeKind::Task, "Task", None, None, 3).await.unwrap();
    store.set_assessed(id, Some("final")).await.unwrap();
    assert!(store.read(id).await.unwrap().is_final());
    store.set_assessed(id, None).await.unwrap();
    assert!(store.read(id).await.unwrap().assessed.is_none());
}

#[tokio::test]
async fn set_files_round_trips_and_clears() {
    let (_dir, store) = store().await;
    let root = store.all().await.unwrap().remove(0).id;
    let id = store.insert(Some(root), NodeKind::Task, "Task", None, None, 3).await.unwrap();
    let files = vec!["src/a.rs".to_owned(), "src/b.rs".to_owned()];
    store.set_files(id, &files).await.unwrap();
    assert_eq!(store.read(id).await.unwrap().files, files);
    assert!(store.set_files(id, &["a b".to_owned()]).await.is_err());
    store.set_files(id, &[]).await.unwrap();
    assert!(store.read(id).await.unwrap().files.is_empty());
}

#[tokio::test]
async fn read_names_a_missing_node() {
    let (_dir, store) = store().await;
    let error = store.read(99).await.unwrap_err();
    assert!(format!("{error:#}").contains("no node 99"), "{error:#}");
}

async fn node_depends(store: &Store, id: i64) -> Vec<i64> {
    store.read(id).await.unwrap().depends_on
}
