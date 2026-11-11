//! The tests of the outline: the marks, the nesting, the fields, and the import into a store.

use super::*;
use crate::store::Store;

const TEXT: &str = "```markdown\n# Demo\n\nSome prose a model added.\n\n## Foundations {c4}\nobjective: the base\nacceptance: `cargo test` passes\n### Storage\n- A store {c2}\n  objective: keep things\n  acceptance: round trip\n  files: src/store.rs src/model.rs\n  - A subtask [done]\n- Another task [done] {c1}\n## Polish\n- Last task\n```\n";

#[test]
fn an_outline_parses_its_items_marks_and_fields() {
    let outline = parse(TEXT).unwrap();
    assert_eq!(outline.title.as_deref(), Some("Demo"));
    let kinds: Vec<NodeKind> = outline.items.iter().map(|i| i.kind).collect();
    assert_eq!(
        kinds,
        [NodeKind::Milestone, NodeKind::Section, NodeKind::Task, NodeKind::Subtask, NodeKind::Task, NodeKind::Milestone, NodeKind::Task]
    );
    let first = &outline.items[0];
    assert_eq!((first.title.as_str(), first.complexity, first.done), ("Foundations", 4, false));
    assert_eq!(first.objective.as_deref(), Some("the base"));
    assert_eq!(first.acceptance.as_deref(), Some("`cargo test` passes"));
    let task = &outline.items[2];
    assert_eq!(task.files, ["src/store.rs", "src/model.rs"]);
    assert!(outline.items[3].done);
    let marked = &outline.items[4];
    assert_eq!((marked.title.as_str(), marked.complexity, marked.done), ("Another task", 1, true));
    assert_eq!(outline.items[6].complexity, 3);
    assert_eq!(outline.summary(), "2 milestones, 1 section, 3 tasks, 1 subtask");
    // the errors: a field before any item, and nothing at all
    assert!(parse("objective: x\n").is_err());
    assert!(parse("# Only a title\n").is_err());
}

#[tokio::test]
async fn an_import_nests_the_items_under_the_root_and_names_the_project() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Untitled").await.unwrap();
    let ids = store.import(1, &parse(TEXT).unwrap()).await.unwrap();
    assert_eq!(ids.len(), 7);
    let nodes = store.all().await.unwrap();
    assert_eq!(nodes[0].title, "Demo", "the root takes the outline's title");
    let by_title = |t: &str| nodes.iter().find(|n| n.title == t).unwrap();
    assert_eq!(by_title("Foundations").parent, Some(1));
    assert_eq!(by_title("Storage").parent, Some(by_title("Foundations").id));
    assert_eq!(by_title("A store").parent, Some(by_title("Storage").id));
    assert_eq!(by_title("A subtask").parent, Some(by_title("A store").id));
    assert_eq!(by_title("Another task").parent, Some(by_title("Storage").id));
    assert_eq!(by_title("Last task").parent, Some(by_title("Polish").id));
    assert_eq!(by_title("A store").files, ["src/store.rs", "src/model.rs"]);
    assert_eq!(by_title("A store").complexity, 2);
    assert_eq!(by_title("Another task").state, crate::model::TaskState::Accepted);
    assert_eq!(by_title("Last task").state, crate::model::TaskState::Defined);
    // the file reads back the same
    let again = Store::open(&dir.path().join("rhipper.md"), "x").await.unwrap();
    assert_eq!(again.all().await.unwrap(), nodes);
}
