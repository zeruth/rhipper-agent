//! The tests of the markdown store: the file round trip and the mutations.

use super::*;
use crate::model::{GambitState, NodeKind};
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
async fn two_changes_of_one_card_read_back_in_order_with_their_sources() {
    let (dir, store) = store().await;
    let store = store.with_source(Source::Console);
    let root = store.all().await.unwrap().remove(0).id;
    let id = store.insert(Some(root), NodeKind::Task, "Task", None, None, 3).await.unwrap();
    let other = store.insert(Some(root), NodeKind::Task, "Other", None, None, 3).await.unwrap();
    store.set_state_by(id, TaskState::InProgress, Source::Run).await.unwrap();
    store.set_state(other, TaskState::Blocked).await.unwrap();
    store.set_state(id, TaskState::Review).await.unwrap();
    // the same state again is no change
    store.set_state(id, TaskState::Review).await.unwrap();
    assert_eq!(store.history_path(), dir.path().join("var").join("history.log"));
    let changes = store.history(Some(id)).unwrap();
    assert_eq!(
        changes.iter().map(|c| (c.from, c.to, c.by)).collect::<Vec<_>>(),
        vec![(TaskState::Defined, TaskState::InProgress, Source::Run), (TaskState::InProgress, TaskState::Review, Source::Console),]
    );
    assert!(changes.iter().all(|c| c.id == id && c.at > 0));
    assert_eq!(store.history(None).unwrap().len(), 3);
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
async fn set_title_trims_and_refuses_an_empty_or_broken_one() {
    let (_dir, store) = store().await;
    let root = store.all().await.unwrap().remove(0).id;
    let id = store.insert(Some(root), NodeKind::Task, "Task", None, None, 3).await.unwrap();
    store.set_title(id, "  Edit a card  ").await.unwrap();
    assert_eq!(store.read(id).await.unwrap().title, "Edit a card");
    assert!(store.set_title(id, "   ").await.is_err());
    assert!(store.set_title(id, "two\nlines").await.is_err());
    assert!(store.set_title(99, "Missing").await.is_err());
    assert_eq!(store.read(id).await.unwrap().title, "Edit a card");
}

#[tokio::test]
async fn set_complexity_round_trips_within_the_range() {
    let (_dir, store) = store().await;
    let root = store.all().await.unwrap().remove(0).id;
    let id = store.insert(Some(root), NodeKind::Task, "Task", None, None, 3).await.unwrap();
    store.set_complexity(id, 5).await.unwrap();
    assert_eq!(store.read(id).await.unwrap().complexity, 5);
    assert!(store.set_complexity(id, 0).await.is_err());
    assert!(store.set_complexity(id, 6).await.is_err());
    assert_eq!(store.read(id).await.unwrap().complexity, 5);
}

#[tokio::test]
async fn set_details_rewrites_the_objective_line() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("rhipper.md");
    let store = Store::open(&path, "Test").await.unwrap();
    let root = store.all().await.unwrap().remove(0).id;
    let id = store.insert(Some(root), NodeKind::Task, "Task", Some("old words."), None, 3).await.unwrap();
    store.set_details(id, Some("new words,\non two lines."), None).await.unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("old words."), "{text}");
    assert_eq!(store.read(id).await.unwrap().objective.as_deref(), Some("new words,\non two lines."));
    // an emptied field leaves the file and reads back as none
    store.set_details(id, Some(""), None).await.unwrap();
    assert!(!std::fs::read_to_string(&path).unwrap().contains("objective:"));
    assert!(store.read(id).await.unwrap().objective.is_none());
}

#[tokio::test]
async fn edit_rewrites_the_named_fields_and_leaves_the_rest() {
    let (_dir, store) = store().await;
    let root = store.all().await.unwrap().remove(0).id;
    let id = store.insert(Some(root), NodeKind::Task, "Task", Some("old words."), Some("it passes."), 3).await.unwrap();
    store.set_state(id, TaskState::InProgress).await.unwrap();
    store.set_notes(id, "the worker's report").await.unwrap();
    store.set_session(id, Some(("sess-1", "abc")), 7).await.unwrap();
    store.edit(id, &Edit { objective: Some("new words."), ..Edit::default() }).await.unwrap();
    let node = store.read(id).await.unwrap();
    assert_eq!(node.objective.as_deref(), Some("new words."));
    assert_eq!(node.acceptance.as_deref(), Some("it passes."));
    assert_eq!(node.title, "Task");
    assert_eq!(node.state, TaskState::InProgress);
    assert_eq!(node.notes.as_deref(), Some("the worker's report"));
    assert_eq!(node.session.as_deref(), Some("sess-1"));
    assert_eq!(node.session_of.as_deref(), Some("abc"));
    assert_eq!(node.session_at, Some(7));
    assert_eq!(node.complexity, 3);

    // every other field, and the session cleared, in one write
    let files = vec!["src/a.rs".to_owned()];
    let edit = Edit {
        title: Some(" Edited "),
        acceptance: Some("it still passes."),
        complexity: Some(5),
        files: Some(&files),
        clear_session: true,
        ..Edit::default()
    };
    store.edit(id, &edit).await.unwrap();
    let node = store.read(id).await.unwrap();
    assert_eq!(node.title, "Edited");
    assert_eq!(node.objective.as_deref(), Some("new words."));
    assert_eq!(node.acceptance.as_deref(), Some("it still passes."));
    assert_eq!(node.complexity, 5);
    assert_eq!(node.files, files);
    assert!(node.session.is_none() && node.session_of.is_none() && node.session_at.is_none());
    assert_eq!(node.notes.as_deref(), Some("the worker's report"));
    assert_eq!(node.state, TaskState::InProgress);

    // a refused field leaves the card as it was
    assert!(store.edit(id, &Edit { title: Some(" "), ..Edit::default() }).await.is_err());
    assert!(store.edit(id, &Edit { complexity: Some(6), ..Edit::default() }).await.is_err());
    assert!(store.edit(id, &Edit { files: Some(&["a b".to_owned()]), ..Edit::default() }).await.is_err());
    assert!(store.edit(99, &Edit { objective: Some("x"), ..Edit::default() }).await.is_err());
    assert_eq!(store.read(id).await.unwrap().title, "Edited");
}

#[tokio::test]
async fn remove_takes_a_leaf_with_its_references_and_a_tree_on_request() {
    let (_dir, store) = store().await;
    let root = store.all().await.unwrap().remove(0).id;
    let phase = store.insert(Some(root), NodeKind::Milestone, "Phase", None, None, 3).await.unwrap();
    let a = store.insert(Some(phase), NodeKind::Task, "A", None, None, 2).await.unwrap();
    let b = store.insert(Some(phase), NodeKind::Task, "B", None, None, 2).await.unwrap();
    store.depend_on(b, a).await.unwrap();
    // a leaf goes, and the card that waited on it no longer does
    assert_eq!(store.remove(a, false).await.unwrap(), [a]);
    assert!(store.read(b).await.unwrap().depends_on.is_empty());
    // a parent is refused without the tree, and the error names the children
    let error = store.remove(phase, false).await.unwrap_err();
    assert!(format!("{error:#}").contains(&format!("#{b}")), "{error:#}");
    assert_eq!(store.remove(phase, true).await.unwrap(), [phase, b]);
    assert_eq!(store.all().await.unwrap().len(), 1);
    // the project stays, and a missing id is named
    assert!(store.remove(root, true).await.is_err());
    assert!(store.remove(99, false).await.is_err());
}

#[tokio::test]
async fn move_node_reparents_a_subtree_in_tree_order_and_refuses_a_cycle() {
    let (_dir, store) = store().await;
    let root = store.all().await.unwrap().remove(0).id;
    let one = store.insert(Some(root), NodeKind::Milestone, "One", None, None, 3).await.unwrap();
    let a = store.insert(Some(one), NodeKind::Task, "A", None, None, 2).await.unwrap();
    let a1 = store.insert(Some(a), NodeKind::Subtask, "A1", None, None, 1).await.unwrap();
    let b = store.insert(Some(one), NodeKind::Task, "B", None, None, 2).await.unwrap();
    let two = store.insert(Some(root), NodeKind::Milestone, "Two", None, None, 3).await.unwrap();
    let c = store.insert(Some(two), NodeKind::Task, "C", None, None, 2).await.unwrap();
    // the subtree reads back under the new parent, last among its children
    store.move_node(a, two, None).await.unwrap();
    let ids: Vec<i64> = store.all().await.unwrap().iter().map(|n| n.id).collect();
    assert_eq!(ids, [root, one, b, two, c, a, a1]);
    assert_eq!(store.read(a).await.unwrap().parent, Some(two));
    assert_eq!(store.read(a1).await.unwrap().parent, Some(a));
    // after a named sibling, with its subtree still behind it
    store.move_node(b, two, Some(a)).await.unwrap();
    let ids: Vec<i64> = store.all().await.unwrap().iter().map(|n| n.id).collect();
    assert_eq!(ids, [root, one, two, c, a, a1, b]);
    // first among the children: right behind the parent's line, its subtree with it
    store.move_first(a, two).await.unwrap();
    let ids: Vec<i64> = store.all().await.unwrap().iter().map(|n| n.id).collect();
    assert_eq!(ids, [root, one, two, a, a1, c, b]);
    store.move_node(a, two, Some(c)).await.unwrap();
    let ids: Vec<i64> = store.all().await.unwrap().iter().map(|n| n.id).collect();
    assert_eq!(ids, [root, one, two, c, a, a1, b]);
    // never under itself or a descendant, never the project, and a sibling under another parent is refused
    assert!(store.move_node(a, a, None).await.is_err());
    assert!(store.move_first(a, a1).await.is_err());
    assert!(store.move_node(two, a1, None).await.is_err());
    assert!(store.move_node(root, two, None).await.is_err());
    assert!(store.move_node(c, one, Some(a)).await.is_err());
    assert!(store.move_node(99, one, None).await.is_err());
    let ids: Vec<i64> = store.all().await.unwrap().iter().map(|n| n.id).collect();
    assert_eq!(ids, [root, one, two, c, a, a1, b]);
}

#[tokio::test]
async fn a_cancelled_card_is_removed_on_set_and_on_open() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("rhipper.md");
    let store = Store::open(&path, "Test").await.unwrap();
    let root = store.all().await.unwrap().remove(0).id;
    let a = store.insert(Some(root), NodeKind::Task, "A", None, None, 2).await.unwrap();
    let b = store.insert(Some(root), NodeKind::Task, "B", None, None, 2).await.unwrap();
    store.set_state(a, TaskState::Cancelled).await.unwrap();
    assert!(store.read(a).await.is_err(), "a cancelled card does not stay");
    // an older file with a cancelled card loses it when it opens
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace(&format!("{b} ‹task› B {{c2}} [defined]"), &format!("{b} ‹task› B {{c2}} [cancelled]"));
    assert!(text.contains("[cancelled]"), "{text}");
    std::fs::write(&path, text).unwrap();
    let again = Store::open(&path, "Test").await.unwrap();
    assert_eq!(again.all().await.unwrap().len(), 1);
}

#[tokio::test]
async fn insert_card_writes_a_complete_card_in_one_go() {
    let (_dir, store) = store().await;
    let root = store.all().await.unwrap().remove(0).id;
    let a = store.insert(Some(root), NodeKind::Task, "A", None, None, 2).await.unwrap();
    let files = vec!["src/a.rs".to_owned(), "src/b.rs".to_owned()];
    let card = Card { objective: Some("do it."), files: &files, waits_on: &[a, a], model: Some("local/qwen"), ..Card::new() };
    let b = store.insert_card(Some(root), NodeKind::Task, "B", &card).await.unwrap();
    let node = store.read(b).await.unwrap();
    assert_eq!(node.files, files);
    assert_eq!(node.depends_on, vec![a], "a dependency named twice is one");
    assert_eq!(node.assignee.as_ref().map(|a| a.model.as_str()), Some("local/qwen"));
    assert_eq!((node.complexity, node.objective.as_deref()), (3, Some("do it.")));
    // a card cannot wait on a node that is not there, name a path with a space or a blank model
    assert!(store.insert_card(Some(root), NodeKind::Task, "C", &Card { waits_on: &[99], ..Card::new() }).await.is_err());
    let spaced = vec!["a b.rs".to_owned()];
    assert!(store.insert_card(Some(root), NodeKind::Task, "C", &Card { files: &spaced, ..Card::new() }).await.is_err());
    assert!(store.insert_card(Some(root), NodeKind::Task, "C", &Card { model: Some(""), ..Card::new() }).await.is_err());
    assert_eq!(store.all().await.unwrap().len(), 3, "a refused card leaves the file alone");
}

#[tokio::test]
async fn a_planned_card_keeps_its_evidence_and_saving_through_a_reopen() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("rhipper.md");
    let id = {
        let store = Store::open(&path, "Test").await.unwrap();
        let root = store.all().await.unwrap().remove(0).id;
        let card = Card { evidence: Some("var/retro.log:41"), saving: Some("two turns a run"), ..Card::new() };
        store.insert_card(Some(root), NodeKind::Task, "Planned", &card).await.unwrap()
    };
    let store = Store::open(&path, "Test").await.unwrap();
    let node = store.read(id).await.unwrap();
    assert_eq!(node.evidence.as_deref(), Some("var/retro.log:41"));
    assert_eq!(node.saving.as_deref(), Some("two turns a run"));
    // a card written by hand carries neither line
    let root = store.read(1).await.unwrap();
    assert_eq!((root.evidence, root.saving), (None, None));
}

#[tokio::test]
async fn read_names_a_missing_node() {
    let (_dir, store) = store().await;
    let error = store.read(99).await.unwrap_err();
    assert!(format!("{error:#}").contains("no node 99"), "{error:#}");
}

#[tokio::test]
async fn gambits_and_a_priority_survive_the_node_mutations() {
    let (_dir, store) = store().await;
    let root = store.all().await.unwrap().remove(0).id;
    let g1 = store.add_gambit("Local first", 4, &["provider".into()]).await.unwrap();
    let g2 = store.add_gambit("Ship the console", 2, &[]).await.unwrap();
    assert_eq!((g1, g2), (1, 2));
    store.set_gambit_state(g2, GambitState::Done).await.unwrap();
    store.set_gambit_weight(g1, 5).await.unwrap();
    store.set_gambit_plan_next(g1, true).await.unwrap();
    // a node mutation rewrites the file and keeps the gambits
    let id = store.insert(Some(root), NodeKind::Task, "Task", None, None, 3).await.unwrap();
    store.set_priority(id, Some(5)).await.unwrap();
    let roadmap = store.load().await.unwrap();
    assert_eq!(roadmap.gambits.len(), 2);
    assert_eq!((roadmap.gambits[0].weight, roadmap.gambits[0].focus.as_slice()), (5, &["provider".to_owned()][..]));
    assert!(roadmap.gambits[0].plan_next, "the plan-next flag survives the rewrite");
    assert!(!roadmap.gambits[1].plan_next);
    assert_eq!(roadmap.gambits[1].state, GambitState::Done, "a done gambit stays in the file");
    assert_eq!(store.read(id).await.unwrap().priority, Some(5));
    assert!(store.set_priority(id, Some(6)).await.is_err());
    assert!(store.add_gambit("Too heavy", 0, &[]).await.is_err());
    assert!(store.set_gambit_state(9, GambitState::Parked).await.unwrap_err().to_string().contains("no gambit g9"));
}

async fn node_depends(store: &Store, id: i64) -> Vec<i64> {
    store.read(id).await.unwrap().depends_on
}
