//! The tests of the setup: the candidates, the hint, the brief, and an init of each kind.

use std::sync::Arc;

use ra_providers::{Completion, CompletionFuture, CompletionRequest, ModelProvider, Usage};

use super::*;

/// A provider that answers every completion with one text.
struct Answers(String);

impl ModelProvider for Answers {
    fn complete(&self, _: CompletionRequest) -> CompletionFuture<'_> {
        let text = self.0.clone();
        Box::pin(async move {
            Ok(Completion { text, usage: Usage::default(), turns: 1, tool_calls: Vec::new(), session_id: None, gotchas: Vec::new() })
        })
    }
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("docs")).unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("README.md"), "# Demo\n\nA demo project.\n\n## Usage\n\nRun it.\n").unwrap();
    std::fs::write(root.join("TODO.md"), "- [x] a store\n- [ ] a parser\n").unwrap();
    std::fs::write(root.join("docs/roadmap-2026.md"), "# Plan\n").unwrap();
    std::fs::write(root.join("docs/notes.md"), "# Notes\n").unwrap();
    std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    dir
}

#[test]
fn the_candidates_are_the_roadmap_shaped_files_and_the_hint_names_them() {
    let dir = project();
    let found: Vec<String> = candidates(dir.path()).iter().map(|p| p.display().to_string().replace('\\', "/")).collect();
    assert_eq!(found, ["TODO.md", "docs/roadmap-2026.md"]);
    assert!(!exists(dir.path()));
    let hint = hint(dir.path());
    assert!(hint[0].starts_with("no rhipper.md in"), "{hint:?}");
    assert!(hint.iter().any(|l| l.contains("rc init --import")), "{hint:?}");
    assert!(hint.last().unwrap().contains("TODO.md, docs/roadmap-2026.md"), "{hint:?}");
    let brief = brief(dir.path());
    assert!(brief.starts_with("# Demo\n"), "{brief}");
    assert!(brief.contains("## README") && brief.contains("## Layout") && brief.contains("src/ — 1 rs"), "{brief}");
    assert!(brief.contains("docs/notes.md:\n  # Notes"), "{brief}");
}

#[tokio::test]
async fn an_init_writes_the_roadmap_the_mode_says_and_refuses_a_second() {
    let dir = project();
    let root = dir.path();
    let outline = "# Demo\n## Foundations {c4}\nobjective: the base\nacceptance: `cargo test` passes\n- A store {c2}\n  objective: keep\n  acceptance: round trip\n  files: src/store.rs\n- A parser\n";
    let worker = Worker::new("claude-opus-5-5").with_provider(Arc::new(Answers(outline.to_owned())));
    let lines = init(root, &worker, Mode::Draft).await.unwrap();
    assert!(lines[0].starts_with("rhipper.md written: 1 milestone, 2 tasks (3 cards) drafted by claude-opus-5-5"), "{lines:?}");
    assert!(exists(root));
    assert!(root.join(ra_context::PROJECT_FILE).exists());
    let store = Store::open(&root.join(ROADMAP_FILE), "x").await.unwrap();
    let nodes = store.all().await.unwrap();
    assert_eq!(nodes[0].title, "Demo");
    assert_eq!(nodes.len(), 4);
    assert_eq!(nodes[2].files, ["src/store.rs"]);
    // a second init refuses
    assert!(init(root, &worker, Mode::Empty).await.is_err());

    // an import reads the file and says so; an empty one names the directory
    let other = project();
    let lines = init(other.path(), &worker, Mode::Import(PathBuf::from("TODO.md"))).await.unwrap();
    assert!(lines[0].contains("imported by"), "{lines:?}");
    let bare = tempfile::tempdir().unwrap();
    let lines = init(bare.path(), &worker, Mode::Empty).await.unwrap();
    assert!(lines[0].contains("and nothing else"), "{lines:?}");
    assert_eq!(Store::open(&bare.path().join(ROADMAP_FILE), "x").await.unwrap().all().await.unwrap().len(), 1);

    // an answer that is no outline is kept, not lost
    let prose = Worker::new("claude-opus-5-5").with_provider(Arc::new(Answers("I would suggest the following plan.".to_owned())));
    let third = project();
    let error = init(third.path(), &prose, Mode::Draft).await.unwrap_err();
    assert!(format!("{error:#}").contains(DRAFT_FILE), "{error:#}");
    assert!(third.path().join(DRAFT_FILE).exists());
    assert!(!exists(third.path()));
}
