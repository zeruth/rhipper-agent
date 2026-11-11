//! The tests of the commands: parsing, the lifecycle names and the tree walk.

use super::*;
use ra_roadmap::store::Store;

/// The worker of the tests, on an OpenRouter model; no test talks to a network.
fn worker() -> Worker {
    Worker::new("z-ai/glm-5.3-flash")
}

#[test]
fn a_model_id_routes_by_its_vendor_prefix() {
    use crate::agent::Route;
    assert_eq!(Route::of("claude-opus-5-5"), Route::ClaudeCode);
    assert_eq!(Route::of("opus"), Route::ClaudeCode);
    assert_eq!(Route::of("z-ai/glm-5.3-flash"), Route::OpenRouter);
    assert_eq!(Route::of("anthropic/claude-sonnet-5.5"), Route::OpenRouter);
}

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
    let (out, _) = run("add task Rhipper parent 1", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["#2 Rhipper"]);
    let (out, _) = run("show", &store, &worker()).await.unwrap();
    assert!(out.iter().any(|l| l.contains("#2 Rhipper")));
}

#[tokio::test]
async fn adds_a_task_under_its_parent() {
    let store = store().await;
    let (out, _) = run("add task Write the decoder parent 1", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["#2 Write the decoder"]);
}

#[tokio::test]
async fn a_task_needs_a_parent() {
    let store = store().await;
    let error = run("add task Orphan", &store, &worker()).await.unwrap_err();
    assert_eq!(format!("{error:#}"), "a node below a project needs parent <id>");
}

#[tokio::test]
async fn a_project_takes_no_parent() {
    let store = store().await;
    let error = run("add project Root parent 7", &store, &worker()).await.unwrap_err();
    assert_eq!(format!("{error:#}"), "a project has no parent");
}

#[tokio::test]
async fn sets_states() {
    let store = store().await;
    run("add task T parent 1", &store, &worker()).await.unwrap();
    let (out, _) = run("set 2 in_progress", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["#2 is now in_progress"]);
    let error = run("set 2 nonsense", &store, &worker()).await.unwrap_err();
    assert!(format!("{error:#}").starts_with("the state is defined, ready, "), "{error:#}");
}

#[tokio::test]
async fn assign_sets_and_clears_the_model_of_a_node() {
    let store = store().await;
    run("add task T parent 1", &store, &worker()).await.unwrap();
    let (out, _) = run("assign 2 claude-haiku-4-5", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["#2 runs on claude-haiku-4-5 through ClaudeCode"]);
    assert_eq!(store.read(2).await.unwrap().assignee.unwrap().model, "claude-haiku-4-5");
    let (out, _) = run("assign 2 none", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["#2 runs on the badge model"]);
    assert!(store.read(2).await.unwrap().assignee.is_none());
}

#[tokio::test]
async fn run_honors_the_model_of_the_node() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("add task T parent 1", &store, &worker()).await.unwrap();
    run("assign 2 z-ai/glm-5.3-flash", &store, &worker()).await.unwrap();
    let provider = Scripted::new(vec![says("done", Vec::new())]);
    // the badge is another OpenRouter model, so the scripted provider serves both; the node's
    // own model is what the request names
    let worker = Worker::new("deepseek/deepseek-v4-flash").with_provider(provider.clone());
    let (out, _) = run("run 2", &store, &worker).await.unwrap();
    assert!(out[0].contains("on glm-5.3-flash"), "{out:?}");
    assert_eq!(provider.seen.lock()[0].model, "z-ai/glm-5.3-flash");
}

#[tokio::test]
async fn files_names_the_paths_and_the_prompt_lists_them() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("add task T parent 1", &store, &worker()).await.unwrap();
    let (out, _) = run("files 2 crates/a/src/lib.rs crates\\b\\src\\main.rs", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["#2 names 2 files: crates/a/src/lib.rs crates/b/src/main.rs"]);
    let (out, _) = run("show 2", &store, &worker()).await.unwrap();
    assert!(out.iter().any(|l| l == "    files: crates/a/src/lib.rs crates/b/src/main.rs"), "{out:?}");

    let provider = Scripted::new(vec![says("done", Vec::new())]);
    let runner = worker().with_provider(provider.clone());
    run("run 2", &store, &runner).await.unwrap();
    let seen = provider.seen.lock();
    let prompt = seen[0].messages.iter().map(|m| m.content.as_str()).collect::<Vec<_>>().join("\n");
    let at = prompt.find("Files:").unwrap_or_else(|| panic!("no Files: in {prompt}"));
    let after = &prompt[at..];
    assert!(after.contains("crates/a/src/lib.rs") && after.contains("crates/b/src/main.rs"), "{prompt}");
    drop(seen);

    let (out, _) = run("files 2", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["#2 names no files"]);
    assert!(store.read(2).await.unwrap().files.is_empty());
}

#[tokio::test]
async fn completes_names() {
    assert_eq!(complete("se"), ["set", "seed"]);
    assert_eq!(complete("q"), ["quit"]);
}

#[tokio::test]
async fn show_indents_by_depth_and_names_a_missing_node() {
    let store = store().await;
    run("add task T parent 1", &store, &worker()).await.unwrap();
    let (out, _) = run("show", &store, &worker()).await.unwrap();
    assert!(out[0].starts_with("#1 "), "{out:?}");
    assert!(out[1].starts_with("  #2 T"), "{out:?}");
    let error = run("show 9", &store, &worker()).await.unwrap_err();
    assert_eq!(format!("{error:#}"), "no node 9");
}

#[tokio::test]
async fn add_names_the_kinds_on_a_bad_one() {
    let store = store().await;
    let error = run("add epic T parent 1", &store, &worker()).await.unwrap_err();
    assert_eq!(format!("{error:#}"), "the kind is project, milestone, section, task or subtask");
}

#[tokio::test]
async fn seed_fills_an_empty_roadmap_and_refuses_a_second_time() {
    let store = store().await;
    let (out, _) = run("seed", &store, &worker()).await.unwrap();
    assert!(out[0].contains("planted"), "{}", out[0]);
    let error = run("seed", &store, &worker()).await.unwrap_err();
    assert!(format!("{error:#}").contains("already holds nodes"), "{error:#}");
    let (out, _) = run("show", &store, &worker()).await.unwrap();
    assert!(out.iter().any(|l| l.contains("ping")), "the seed holds the ping task: {out:?}");
}

/// A provider that plays a script: each completion pops the next answer. It records the requests
/// it saw, so a test can read what the loop sent.
struct Scripted {
    answers: parking_lot::Mutex<Vec<ra_providers::Completion>>,
    seen: parking_lot::Mutex<Vec<ra_providers::CompletionRequest>>,
}

impl Scripted {
    fn new(answers: Vec<ra_providers::Completion>) -> std::sync::Arc<Self> {
        let mut answers = answers;
        answers.reverse();
        std::sync::Arc::new(Self { answers: parking_lot::Mutex::new(answers), seen: parking_lot::Mutex::new(Vec::new()) })
    }
}

impl ra_providers::ModelProvider for Scripted {
    fn complete(&self, request: ra_providers::CompletionRequest) -> ra_providers::CompletionFuture<'_> {
        self.seen.lock().push(request);
        let next = self.answers.lock().pop();
        Box::pin(async move { next.ok_or_else(|| ra_providers::ProviderError::Other("the script ran out".into())) })
    }
}

fn says(text: &str, calls: Vec<ra_providers::ToolCall>) -> ra_providers::Completion {
    let usage = ra_providers::Usage { prompt_tokens: 100, completion_tokens: 10, cached_tokens: 0, cost_usd: Some(0.01) };
    ra_providers::Completion { text: text.to_owned(), usage, tool_calls: calls, session_id: None, gotchas: Vec::new() }
}

#[tokio::test]
async fn run_on_a_bare_model_loops_through_the_tools_and_lands_the_answer() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("add task Write the greeting parent 1", &store, &worker()).await.unwrap();
    let write = ra_providers::ToolCall {
        id: "call_1".into(),
        name: "write_file".into(),
        arguments: r#"{"path":"hello.txt","content":"hello from the worker\n"}"#.into(),
    };
    let read = ra_providers::ToolCall { id: "call_2".into(), name: "read_file".into(), arguments: r#"{"path":"hello.txt"}"#.into() };
    let provider =
        Scripted::new(vec![says("", vec![write]), says("checking", vec![read]), says("wrote hello.txt and read it back", vec![])]);
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(provider.clone());

    let (out, _) = run("run 2", &store, &worker).await.unwrap();
    assert!(out[0].contains("3 turn(s), 2 tool call(s)"), "{out:?}");
    assert!(out[0].contains("$0.03"), "{out:?}");
    assert_eq!(std::fs::read_to_string(dir.path().join("hello.txt")).unwrap(), "hello from the worker\n");
    let node = store.read(2).await.unwrap();
    assert_eq!(node.state, ra_roadmap::model::TaskState::Implemented);
    assert_eq!(node.notes.as_deref(), Some("wrote hello.txt and read it back"));
    assert_eq!(worker.tally().calls, 3);

    // the loop sent the tools, the workspace, and the tool results in order
    let seen = provider.seen.lock();
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[0].tools.len(), 6);
    assert_eq!(seen[0].workspace.as_deref().map(|p| p.canonicalize().unwrap()), Some(dir.path().canonicalize().unwrap()));
    let last = &seen[2].messages;
    assert_eq!(last[last.len() - 1].role, "tool");
    assert!(last[last.len() - 1].content.starts_with("    1│hello from the worker"), "{}", last[last.len() - 1].content);
    assert_eq!(last[last.len() - 3].tool_call_id.as_deref(), Some("call_1"));
}

/// The answer of a reassessment that keeps the text: the three lines the prompt asks for.
fn keeps(objective: &str, acceptance: &str) -> ra_providers::Completion {
    says(&format!("overview: already sharp\nobjective: {objective}\nacceptance: {acceptance}"), Vec::new())
}

/// A store with one task, #2, that holds an objective and an acceptance.
async fn store_with_a_task() -> Store {
    let store = store().await;
    store.insert(Some(1), ra_roadmap::model::NodeKind::Task, "T", Some("do it."), Some("it is done."), 3).await.unwrap();
    store
}

#[tokio::test]
async fn a_second_reassess_of_an_unchanged_node_makes_no_call() {
    let store = store_with_a_task().await;
    let provider = Scripted::new(vec![keeps("do it.", "it is done.")]);
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(provider.clone());
    let (out, _) = run("reassess 2", &store, &worker).await.unwrap();
    assert!(out[0].contains("0 unchanged"), "{out:?}");
    let assessed = store.read(2).await.unwrap().assessed.unwrap();
    assert_eq!(assessed.len(), 16, "{assessed}");
    assert!(assessed.chars().all(|c| c.is_ascii_hexdigit()), "{assessed}");

    let (out, _) = run("reassess 2", &store, &worker).await.unwrap();
    assert_eq!(provider.seen.lock().len(), 1, "the second pass made no call");
    assert!(out[0].contains("1 unchanged"), "{out:?}");
    assert!(out.iter().any(|l| l.contains("#2 T assessed and unchanged")), "{out:?}");
    assert!(!out.iter().any(|l| l.starts_with('⚠')), "{out:?}");
}

#[tokio::test]
async fn a_final_node_is_skipped_with_no_call() {
    let store = store_with_a_task().await;
    let (out, _) = run("final 2", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["#2 is final: no reassessment rewrites it"]);
    let (out, _) = run("show", &store, &worker()).await.unwrap();
    assert!(out.iter().any(|l| l.contains("#2 T [defined] [final]")), "{out:?}");

    let provider = Scripted::new(Vec::new());
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(provider.clone());
    // the project above still calls, and the empty script turns that into a warning line
    let (out, _) = run("reassess-tree 1", &store, &worker).await.unwrap();
    assert!(provider.seen.lock().iter().all(|r| !r.messages.iter().any(|m| m.content.contains("Task #2:"))), "#2 made no call");
    assert!(out.iter().any(|l| l.contains("#2 T [final] assessed and unchanged")), "{out:?}");
    assert_eq!(store.read(2).await.unwrap().objective.as_deref(), Some("do it."));

    let (out, _) = run("final 2 off", &store, &worker).await.unwrap();
    assert_eq!(out, vec!["#2 is no longer final"]);
    assert!(store.read(2).await.unwrap().assessed.is_none());
    let (out, _) = run("show", &store, &worker).await.unwrap();
    assert!(!out.iter().any(|l| l.contains("[final]")), "{out:?}");
}

#[tokio::test]
async fn a_final_single_node_makes_no_call() {
    let store = store_with_a_task().await;
    run("final 2", &store, &worker()).await.unwrap();
    let provider = Scripted::new(Vec::new());
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(provider.clone());
    let (out, _) = run("reassess 2", &store, &worker).await.unwrap();
    assert!(provider.seen.lock().is_empty(), "no provider call");
    assert!(out[0].contains("1 unchanged"), "{out:?}");
    assert!(store.read(2).await.unwrap().is_final(), "the mark stays");
}

#[tokio::test]
async fn editing_the_objective_after_an_assessment_calls_again() {
    let store = store_with_a_task().await;
    let provider = Scripted::new(vec![keeps("do it.", "it is done."), keeps("do it twice.", "it is done twice.")]);
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(provider.clone());
    run("reassess 2", &store, &worker).await.unwrap();
    let first = store.read(2).await.unwrap().assessed.unwrap();
    store.set_details(2, Some("do it twice."), None).await.unwrap();

    let (out, _) = run("reassess 2", &store, &worker).await.unwrap();
    assert_eq!(provider.seen.lock().len(), 2, "the edit made the next pass call");
    assert!(out[0].contains("0 unchanged"), "{out:?}");
    let node = store.read(2).await.unwrap();
    assert_eq!(node.acceptance.as_deref(), Some("it is done twice."));
    assert_ne!(node.assessed.unwrap(), first, "the fingerprint follows the new text");
}

#[tokio::test]
async fn run_on_a_bare_model_fails_the_node_when_the_provider_does() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("add task T parent 1", &store, &worker()).await.unwrap();
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(Scripted::new(Vec::new()));
    let error = run("run 2", &store, &worker).await.unwrap_err();
    assert!(format!("{error:#}").contains("the script ran out"), "{error:#}");
    assert_eq!(store.read(2).await.unwrap().state, ra_roadmap::model::TaskState::Failed);
}

#[test]
fn the_complexity_sets_the_budget() {
    use crate::agent::budget_for;
    assert_eq!([1, 2, 3, 4, 5].map(budget_for), [0.25, 0.50, 1.00, 2.00, 5.00]);
}

/// The script behind a harness: it brings its own tools, as Claude Code does.
struct Harness(std::sync::Arc<Scripted>);

impl ra_providers::ModelProvider for Harness {
    fn complete(&self, request: ra_providers::CompletionRequest) -> ra_providers::CompletionFuture<'_> {
        self.0.complete(request)
    }

    fn has_own_tools(&self) -> bool {
        true
    }
}

#[tokio::test]
async fn a_second_run_of_a_node_resumes_its_session() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("add task T parent 1", &store, &worker()).await.unwrap();
    let mut first = says("done", vec![]);
    first.session_id = Some("sess-1".into());
    let script = Scripted::new(vec![first, says("done again", vec![])]);
    let runner = Worker::new("claude-opus-5-5").with_provider(std::sync::Arc::new(Harness(script.clone())));
    run("run 2", &store, &runner).await.unwrap();
    assert_eq!(store.read(2).await.unwrap().session.as_deref(), Some("sess-1"));
    run("run 2", &store, &runner).await.unwrap();
    let seen = script.seen.lock();
    assert_eq!(seen[0].resume, None, "a first run has nothing to resume");
    assert_eq!(seen[1].resume.as_deref(), Some("sess-1"), "the second run resumes the first");
}

#[tokio::test]
async fn a_run_past_its_budget_stops_and_fails_the_node_with_a_note() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    // c3: a $1.00 budget
    run("add task T parent 1", &store, &worker()).await.unwrap();
    let read = |n: &str| ra_providers::ToolCall { id: n.into(), name: "list_dir".into(), arguments: r#"{"path":"."}"#.into() };
    let costly = |calls| {
        let mut answer = says("working", calls);
        answer.usage.cost_usd = Some(0.6);
        answer
    };
    let provider = Scripted::new(vec![costly(vec![read("c1")]), costly(vec![read("c2")]), says("never reached", vec![])]);
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(provider.clone());
    let error = run("run 2", &store, &runner).await.unwrap_err();
    assert!(format!("{error:#}").contains("budget"), "{error:#}");
    assert_eq!(provider.seen.lock().len(), 2, "the loop stopped at the turn that crossed the budget");
    assert_eq!(provider.seen.lock()[0].budget_usd, Some(1.0));
    let node = store.read(2).await.unwrap();
    assert_eq!(node.state, ra_roadmap::model::TaskState::Failed);
    let note = node.notes.unwrap();
    assert!(note.contains("budget") && note.contains("$1.00") && note.contains("$1.2000"), "{note}");

    // --budget overrides the complexity
    run("add task U parent 1", &store, &worker()).await.unwrap();
    let provider = Scripted::new(vec![says("done", vec![])]);
    let capped = Worker::new("z-ai/glm-5.3-flash").with_provider(provider.clone()).with_budget(Some(0.1));
    run("run 3", &store, &capped).await.unwrap();
    assert_eq!(provider.seen.lock()[0].budget_usd, Some(0.1));
}

#[tokio::test]
async fn run_refuses_a_project_and_names_a_missing_key() {
    let store = store().await;
    let error = run("run 1", &store, &worker()).await.unwrap_err();
    assert!(format!("{error:#}").contains("is a project"), "{error:#}");
    run("add task T parent 1", &store, &worker()).await.unwrap();
    // this machine may hold a stored key or a real one in the environment; when neither source
    // holds a key, the run must refuse with the key's name. The env lock covers the read and the
    // removal, so a parallel secret test cannot flip the variable in between.
    let guard = crate::secret::ENV_LOCK.lock();
    let key_configured = std::env::var("OPENROUTER_API_KEY").map_or(false, |k| !k.trim().is_empty()) || crate::secret::has_key();
    if !key_configured {
        // SAFETY: the test holds the env lock; no parallel test reads the key
        unsafe { std::env::remove_var("OPENROUTER_API_KEY") };
        let error = run("run 2", &store, &worker()).await.unwrap_err();
        assert!(format!("{error:#}").contains("OPENROUTER_API_KEY") || format!("{error:#}").contains("key"), "{error:#}");
    }
    drop(guard);
}
