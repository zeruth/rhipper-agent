//! A run a cap ended before the worker reported leaves the agent's own handoff: outcome `cut
//! off`, the files the tree shows changed, and the tail of the worker's last message, which the
//! next run's prompt starts from. A run the project's context cap ends continues, its handoff
//! carrying the same files and tail.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ra_console::agent::Worker;
use ra_context::{CONTINUES, CUT_OFF, Handoff, TaskContext};
use ra_providers::claude_code::result_of_stream;
use ra_providers::{Completion, CompletionFuture, CompletionRequest, ModelProvider, ProviderError, TurnUsage, Usage};
use ra_roadmap::model::NodeKind;
use ra_roadmap::store::Store;
use ra_usage::UsageStore;

/// A worker's last message, long enough that its head is clipped.
fn long_message() -> String {
    let mut text: Vec<String> = (1..=30).map(|n| format!("step {n}: read the store")).collect();
    text.push("the parser is done; the store's two literals are left".to_owned());
    text.join("\n")
}

#[test]
fn a_cut_off_run_leaves_its_changed_files_and_the_tail_of_its_last_message() {
    let changed = vec!["crates/ra-roadmap/src/markdown.rs".to_owned(), "crates/ra-roadmap/src/store.rs".to_owned()];
    let handoff = Handoff::cut_off(1_700_000_000, "claude-opus-5-5", "the turn cap of 40", changed.clone(), &long_message());
    assert_eq!(handoff.outcome, CUT_OFF);
    assert_eq!(handoff.changed, changed);
    assert!(handoff.check.contains("the turn cap of 40"), "{}", handoff.check);
    assert!(handoff.report.contains("the store's two literals are left"), "{}", handoff.report);
    assert!(!handoff.report.contains("step 1:"), "the head of the message is clipped: {}", handoff.report);
    assert!(handoff.report.contains("crates/ra-roadmap/src/store.rs"), "{}", handoff.report);
}

#[test]
fn the_next_runs_prompt_starts_from_the_cut_off_handoff() {
    let root = tempfile::tempdir().unwrap();
    let mut task = TaskContext::open(root.path(), 7);
    assert!(task.handoffs.is_empty());
    let changed = vec!["src/lib.rs".to_owned()];
    task.record(Handoff::cut_off(1_700_000_000, "claude-opus-5-5", "the context cap of 160k tokens", changed.clone(), &long_message()));
    task.save(root.path()).unwrap();

    let next = TaskContext::open(root.path(), 7);
    let last = next.handoffs.last().expect("the cut-off handoff is kept");
    assert_eq!(last.outcome, CUT_OFF);
    assert_eq!(last.changed, changed);
    let prompt = next.prompt();
    assert!(prompt.contains("continue from the last one"), "{prompt}");
    assert!(prompt.contains(&format!("claude-opus-5-5 · {CUT_OFF}")), "{prompt}");
    assert!(prompt.contains("changed: src/lib.rs"), "{prompt}");
    assert!(prompt.contains("the store's two literals are left"), "{prompt}");
}

#[test]
fn a_cut_off_run_that_said_nothing_says_so() {
    let handoff = Handoff::cut_off(1, "m", "the turn cap of 2", Vec::new(), "  \n");
    assert!(handoff.report.contains("changed: nothing in the tree"), "{}", handoff.report);
    assert!(handoff.report.contains("the worker's last message: none"), "{}", handoff.report);
}

/// A harness the project's context cap stops after it wrote a file: the run ends on the cap
/// with the worker's long last message and no report.
struct CappedAtContext {
    root: PathBuf,
}

impl ModelProvider for CappedAtContext {
    fn complete(&self, _request: CompletionRequest) -> CompletionFuture<'_> {
        std::fs::write(self.root.join("src/store.rs"), "pub fn store() {}\n").unwrap();
        let usage = Usage { prompt_tokens: 400_000, completion_tokens: 500, cached_tokens: 390_000, cost_usd: None, cost_computed: false };
        let completion = Completion {
            text: long_message(),
            usage,
            turns: 5,
            turn_usage: vec![TurnUsage::of(&usage)],
            tool_calls: Vec::new(),
            session_id: None,
            gotchas: Vec::new(),
            reads: Vec::new(),
            read_in: 400_000,
            whole_reads: 0,
        };
        Box::pin(async move { Err(ProviderError::ContextOut { completion: Box::new(completion), cap: 120_000, size: 131_000 }) })
    }

    fn has_own_tools(&self) -> bool {
        true
    }
}

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git").args(args).current_dir(dir).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

#[tokio::test]
async fn a_run_the_context_cap_ends_continues_with_its_changed_files_and_the_tail_of_its_last_message() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"probe_cut\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "#[test]\nfn probe() {}\n").unwrap();
    std::fs::write(dir.path().join("rhipper.toml"), "[worker]\ncontext = 120000\n").unwrap();
    // a committed tree, so the file the run adds shows by name and not under an untracked `src/`
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["-c", "user.name=test", "-c", "user.email=test@example.com", "commit", "-q", "-m", "probe"]);
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    let id = store.insert(Some(1), NodeKind::Task, "Probe", None, None, 3).await.unwrap();
    let provider = Arc::new(CappedAtContext { root: dir.path().to_path_buf() });
    let worker = Worker::new("claude-opus-5-5").with_provider(provider).with_ledger(UsageStore::in_memory().await.unwrap());

    let lines = ra_console::agent::run(&store, id, &worker).await.unwrap();
    assert!(lines.iter().any(|line| line.starts_with(&format!("↻ #{id} Probe continues"))), "{lines:?}");
    let task = TaskContext::open(dir.path(), id);
    let last = task.handoffs.last().expect("the capped run leaves a handoff");
    assert_eq!(last.outcome, CONTINUES);
    assert!(last.changed.contains(&"src/store.rs".to_owned()), "{:?}", last.changed);
    assert!(last.report.contains("the context cap of 120k tokens (a turn of 131k) ended the run after 5 turn(s)"), "{}", last.report);
    assert!(last.report.contains("src/store.rs"), "{}", last.report);
    assert!(last.report.contains("the worker's last message, its tail:"), "{}", last.report);
    assert!(last.report.ends_with("the store's two literals are left"), "{}", last.report);
    assert!(!last.report.contains("step 1:"), "the head of the message is clipped: {}", last.report);
}

#[test]
fn a_claude_code_run_out_of_turns_carries_the_words_of_its_last_turn_with_text() {
    let stream = [
        r#"{"type":"assistant","message":{"id":"m1","usage":{"input_tokens":10,"output_tokens":5},"content":[{"type":"text","text":"the parser is done; the store's two literals are left"}]}}"#,
        r#"{"type":"assistant","message":{"id":"m2","usage":{"input_tokens":20,"output_tokens":5},"content":[{"type":"tool_use","name":"Read","input":{"file_path":"src/store.rs","offset":1,"limit":20}}]}}"#,
        r#"{"type":"result","subtype":"error_max_turns","is_error":true,"num_turns":2,"result":""}"#,
    ]
    .join("\n");
    match result_of_stream(&stream) {
        Err(ProviderError::TurnsOut(completion)) => {
            assert_eq!(completion.text, "the parser is done; the store's two literals are left");
        }
        other => panic!("a run out of turns: {other:?}"),
    }
    // a run that finished keeps its own result
    let finished = stream.replace(
        r#""subtype":"error_max_turns","is_error":true,"num_turns":2,"result":"""#,
        r#""subtype":"success","is_error":false,"num_turns":2,"result":"the report""#,
    );
    assert_eq!(result_of_stream(&finished).unwrap().text, "the report");
}
