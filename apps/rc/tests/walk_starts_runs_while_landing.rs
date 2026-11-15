//! A walk that starts with an implemented card waiting to land starts its runs at once: the
//! landing waits beside them (here on the hub's lock on landing, held by the test until the first
//! run's first call) and does not hold the runners idle before them.

use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use ra_console::agent::{Worker, run_parallel};
use ra_roadmap::history::Source;
use ra_roadmap::model::{NodeKind, TaskState};
use ra_roadmap::store::Store;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(["-c", "user.name=test", "-c", "user.email=test@example.com"])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn says(text: &str, calls: Vec<ra_providers::ToolCall>) -> ra_providers::Completion {
    let usage =
        ra_providers::Usage { prompt_tokens: 100, completion_tokens: 10, cached_tokens: 0, cost_usd: Some(0.01), cost_computed: false };
    ra_providers::Completion {
        text: text.to_owned(),
        usage,
        turns: 1,
        turn_usage: vec![ra_providers::TurnUsage::of(&usage)],
        tool_calls: calls,
        session_id: None,
        gotchas: Vec::new(),
        reads: Vec::new(),
        read_in: 100,
        whole_reads: 0,
    }
}

/// A provider that lets go of the landing lock the first time any run calls it, and answers the
/// worker with a write and then `done`, the reviewer with `accepted`.
struct ReleasesOnFirstCall {
    held: Mutex<Option<ra_console::hub::Guard>>,
    released: AtomicBool,
}

impl ra_providers::ModelProvider for ReleasesOnFirstCall {
    fn complete(&self, request: ra_providers::CompletionRequest) -> ra_providers::CompletionFuture<'_> {
        if self.held.lock().expect("the lock").take().is_some() {
            self.released.store(true, Ordering::SeqCst);
        }
        let answer = if request.tools.is_empty() {
            says("accepted", vec![])
        } else if request.messages.iter().any(|m| m.role == "tool") {
            says("done", vec![])
        } else {
            let write = ra_providers::ToolCall {
                id: "call_1".into(),
                name: "write_file".into(),
                arguments: r#"{"path":"made.txt","content":"by the worker\n"}"#.into(),
            };
            says("", vec![write])
        };
        Box::pin(async move { Ok(answer) })
    }
}

#[tokio::test]
async fn a_walk_starts_runs_while_its_startup_landing_waits() {
    let hub = tempfile::tempdir().expect("a hub dir");
    // SAFETY: the only test of this binary, and nothing else runs yet that reads the environment
    unsafe { std::env::set_var("RHIPPER_HUB", hub.path()) };
    let dir = tempfile::tempdir().expect("a tempdir");
    std::fs::write(dir.path().join("a.txt"), "a").expect("a file");
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "probe"]);
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper Agent").await.expect("the store");
    let waiting = store.insert(Some(1), NodeKind::Task, "Waiting", None, None, 3).await.expect("a card");
    let ready = store.insert(Some(1), NodeKind::Task, "Ready", None, None, 3).await.expect("a card");
    store.set_state_by(waiting, TaskState::Implemented, Source::Run).await.expect("the state");

    // the landing of the implemented card waits on this lock until a run has started
    let name = format!("land-{}", ra_console::hub::project_name(&ra_console::hub::project_root_of(dir.path())).to_ascii_lowercase());
    let held = ra_console::hub::try_guard(&name).expect("the lock opens").expect("the lock is free");
    let provider = Arc::new(ReleasesOnFirstCall { held: Mutex::new(Some(held)), released: AtomicBool::new(false) });
    let check: ra_tools::Script = Arc::new(|_: &str| Ok((0, "ok\n".to_owned())));
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(provider.clone()).with_script(check);

    // a walk that lands before it runs never gets to the first call, and the lock is never let go
    let walked = tokio::time::timeout(std::time::Duration::from_secs(120), run_parallel(&store, &runner, 2, false))
        .await
        .expect("the walk started its run while the landing waited");
    let out = walked.expect("the walk ends");
    assert!(provider.released.load(Ordering::SeqCst), "{out:?}");
    assert_eq!(store.read(waiting).await.expect("the card reads").state, TaskState::Accepted, "{out:?}");
    assert_ne!(store.read(ready).await.expect("the card reads").state, TaskState::Defined, "{out:?}");
}
