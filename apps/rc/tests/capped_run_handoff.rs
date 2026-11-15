//! A run cut off without a report leaves a handoff of what the agent holds: the files the tree
//! shows changed, the last shell command with how it ended, and the tail of the worker's last
//! message, so the next run starts from them and not from "no report".

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ra_console::agent::Worker;
use ra_context::{CUT_OFF, TaskContext};
use ra_providers::claude_code::{STREAM_FILE, last_command};
use ra_providers::{Completion, CompletionFuture, CompletionRequest, ModelProvider, ProviderError, TurnUsage, Usage};
use ra_roadmap::model::NodeKind;
use ra_roadmap::store::Store;
use ra_usage::UsageStore;

/// The stream of a run whose last turn ran a failing test and whose run then ran out of turns.
fn stream() -> String {
    [
        r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo build -q"}}]}}"#,
        r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"","is_error":false}]}}"#,
        r#"{"type":"assistant","message":{"id":"m2","content":[{"type":"tool_use","id":"t2","name":"Read","input":{"file_path":"src/lib.rs"}}]}}"#,
        r#"{"type":"assistant","message":{"id":"m3","content":[{"type":"tool_use","id":"t3","name":"Bash","input":{"command":"cargo test -q probe"}}]}}"#,
        r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t3","content":[{"type":"text","text":"Exit code 101\nthread 'probe' panicked"}],"is_error":true}]}}"#,
        r#"{"type":"result","subtype":"error_max_turns","is_error":true,"num_turns":3,"result":""}"#,
    ]
    .join("\n")
}

#[test]
fn the_last_shell_command_of_a_stream_carries_its_exit_status() {
    assert_eq!(last_command(&stream()).as_deref(), Some("`cargo test -q probe` · exit 101"));
    let unanswered = stream().replace(r#""tool_use_id":"t3""#, r#""tool_use_id":"t9""#);
    assert_eq!(
        last_command(&unanswered).as_deref(),
        Some("`cargo test -q probe` · no result: the run ended before it came back"),
        "a command the run ended on has no result"
    );
    let first: String = stream().lines().take(2).collect::<Vec<_>>().join("\n");
    assert_eq!(last_command(&first).as_deref(), Some("`cargo build -q` · exit 0"));
    assert_eq!(last_command(r#"{"type":"result","subtype":"success"}"#), None, "a run with no shell command names none");
}

/// A harness the hard turn cap stops after it wrote a file and ran a failing test: the run ends
/// out of turns with the worker's words and no report.
struct OutOfTurns {
    root: PathBuf,
}

impl ModelProvider for OutOfTurns {
    fn complete(&self, _request: CompletionRequest) -> CompletionFuture<'_> {
        std::fs::write(self.root.join("src/store.rs"), "pub fn store() {}\n").unwrap();
        let path = self.root.join(STREAM_FILE);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, stream()).unwrap();
        let usage = Usage { prompt_tokens: 40_000, completion_tokens: 500, cached_tokens: 30_000, cost_usd: None, cost_computed: false };
        let completion = Completion {
            text: "the parser is done; the store's test still fails".to_owned(),
            usage,
            turns: 40,
            turn_usage: vec![TurnUsage::of(&usage)],
            tool_calls: Vec::new(),
            session_id: None,
            gotchas: Vec::new(),
            reads: Vec::new(),
            read_in: 40_000,
            whole_reads: 0,
            whole_files: Vec::new(),
        };
        Box::pin(async move { Err(ProviderError::TurnsOut(Box::new(completion))) })
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
async fn a_run_out_of_turns_leaves_its_changed_files_its_last_command_and_its_last_words() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"probe_capped\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "#[test]\nfn probe() {}\n").unwrap();
    std::fs::write(dir.path().join(".gitignore"), "var/\nrhipper.md\n").unwrap();
    // a committed tree, so the file the run adds shows by name and not under an untracked `src/`
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["-c", "user.name=test", "-c", "user.email=test@example.com", "commit", "-q", "-m", "probe"]);
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    let id = store.insert(Some(1), NodeKind::Task, "Probe", None, None, 3).await.unwrap();
    let provider = Arc::new(OutOfTurns { root: dir.path().to_path_buf() });
    let worker = Worker::new("claude-opus-5-5").with_provider(provider).with_ledger(UsageStore::in_memory().await.unwrap());

    let ended = ra_console::agent::run(&store, id, &worker).await;
    assert!(ended.is_err(), "a run out of turns fails: {ended:?}");
    let task = TaskContext::open(dir.path(), id);
    let last = task.handoffs.last().expect("the cut-off run leaves a handoff");
    assert_eq!(last.outcome, CUT_OFF);
    assert!(last.changed.contains(&"src/store.rs".to_owned()), "{:?}", last.changed);
    assert!(last.report.contains("src/store.rs"), "{}", last.report);
    assert!(last.report.contains("last command: `cargo test -q probe` · exit 101"), "{}", last.report);
    assert!(last.report.ends_with("the parser is done; the store's test still fails"), "{}", last.report);
    // the next run's prompt starts from it
    let prompt = task.prompt();
    assert!(prompt.contains(&format!("claude-opus-5-5 · {CUT_OFF}")), "{prompt}");
    assert!(prompt.contains("last command: `cargo test -q probe` · exit 101"), "{prompt}");
}
