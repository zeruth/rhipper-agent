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

/// A plain outline, as a person types it: headings, bullets, an indented line as the objective
/// and `accept:` as the acceptance.
const PLAIN: &str = "## Parsing\n- Read the headings\n  Headings become milestones and sections.\n  accept: two headings give two nodes\n- Read the bullets\n  A bullet is a task.\n- Read the fields\n## Command\n- Add the command\n  accept: `rc import` adds the nodes\n- Print the dry run\n";

#[test]
fn a_plain_outline_reads_an_indented_line_as_the_objective_and_accept_as_the_acceptance() {
    let outline = parse(PLAIN).unwrap();
    assert_eq!(outline.items.len(), 7);
    let first = &outline.items[1];
    assert_eq!(first.title, "Read the headings");
    assert_eq!(first.objective.as_deref(), Some("Headings become milestones and sections."));
    assert_eq!(first.acceptance.as_deref(), Some("two headings give two nodes"));
    assert_eq!(outline.items[2].objective.as_deref(), Some("A bullet is a task."));
    assert_eq!(outline.items[2].acceptance, None);
    assert_eq!(outline.items[3].objective, None);
    assert_eq!(outline.items[5].acceptance.as_deref(), Some("`rc import` adds the nodes"));
    // two indented lines join as one objective; a line at the margin stays prose
    let joined = parse("- One\n  first line\n  second line\nprose at the margin\n").unwrap();
    assert_eq!(joined.items[0].objective.as_deref(), Some("first line second line"));
    let preview = outline.preview();
    assert_eq!(preview[0], "milestone Parsing {c3}");
    assert_eq!(preview[1], "  task Read the headings {c3}");
    assert_eq!(preview[2], "      objective: Headings become milestones and sections.");
    assert_eq!(preview[3], "      acceptance: two headings give two nodes");
    assert_eq!(preview.len(), 7 + 4, "seven items and their four detail lines");
}

#[tokio::test]
async fn a_plain_outline_of_two_headings_and_five_bullets_adds_seven_nodes() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Demo").await.unwrap();
    let ids = store.import(1, &parse(PLAIN).unwrap()).await.unwrap();
    assert_eq!(ids.len(), 7);
    let nodes = store.all().await.unwrap();
    assert_eq!(nodes.len(), 8, "the root and the seven");
    assert_eq!(nodes[0].title, "Demo", "a plain outline names no title, so the root keeps its own");
    let by_title = |t: &str| nodes.iter().find(|n| n.title == t).unwrap();
    let (parsing, command) = (by_title("Parsing"), by_title("Command"));
    assert_eq!((parsing.kind, parsing.parent), (NodeKind::Milestone, Some(1)));
    assert_eq!((command.kind, command.parent), (NodeKind::Milestone, Some(1)));
    for title in ["Read the headings", "Read the bullets", "Read the fields"] {
        assert_eq!((by_title(title).kind, by_title(title).parent), (NodeKind::Task, Some(parsing.id)), "{title}");
    }
    for title in ["Add the command", "Print the dry run"] {
        assert_eq!((by_title(title).kind, by_title(title).parent), (NodeKind::Task, Some(command.id)), "{title}");
    }
    let headings = by_title("Read the headings");
    assert_eq!(headings.objective.as_deref(), Some("Headings become milestones and sections."));
    assert_eq!(headings.acceptance.as_deref(), Some("two headings give two nodes"));
    assert_eq!(by_title("Add the command").acceptance.as_deref(), Some("`rc import` adds the nodes"));
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
