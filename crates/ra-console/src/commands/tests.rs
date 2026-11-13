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
    assert_eq!(Route::of("local/qwen3"), Route::Local);
    // only the prefix routes locally: a vendor named local-something is still OpenRouter
    assert_eq!(Route::of("local-ai/qwen3"), Route::OpenRouter);
    assert_eq!(Route::of("local"), Route::ClaudeCode);
}

#[tokio::test]
async fn a_short_name_resolves_to_the_current_claude_id_wherever_a_model_is_typed() {
    use crate::agent::{ALIASES, Route, resolve};
    for (alias, id) in ALIASES {
        assert_eq!(resolve(alias), id);
        assert_eq!(Route::of(id), Route::ClaudeCode, "{id}");
    }
    assert_eq!(resolve("Haiku "), "claude-haiku-4-5-20251001");
    assert_eq!(resolve("z-ai/glm-5.3-flash"), "z-ai/glm-5.3-flash");
    assert_eq!(Worker::new("haiku").model, "claude-haiku-4-5-20251001");
    let store = store().await;
    let (out, outcome) = run("/model opus", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["the next worker session runs on claude-opus-5-5 through Claude Code"]);
    assert!(matches!(outcome, Outcome::Model(model) if model == "claude-opus-5-5"));
    run("/add task T parent 1", &store, &worker()).await.unwrap();
    let (out, _) = run("/assign 2 sonnet", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["#2 runs on claude-sonnet-5-5 through Claude Code"]);
    assert_eq!(store.read(2).await.unwrap().assignee.unwrap().model, "claude-sonnet-5-5");
}

#[tokio::test]
async fn a_ping_on_the_local_route_with_nothing_listening_names_the_url() {
    // a port that was just freed, so the test never depends on what runs on 8080 here
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://127.0.0.1:{}/v1", listener.local_addr().unwrap().port());
    drop(listener);
    // the env lock covers the variable for the whole ping, as the secret tests do for the key
    let guard = crate::secret::ENV_LOCK.lock();
    let before = std::env::var(ra_providers::openrouter::LOCAL_URL_VAR).ok();
    // SAFETY: the test holds the env lock; no parallel test reads the variable
    unsafe { std::env::set_var(ra_providers::openrouter::LOCAL_URL_VAR, &url) };
    let error = run("/ping", &store().await, &Worker::new("local/x")).await.unwrap_err();
    // SAFETY: still under the env lock
    unsafe {
        match before {
            Some(value) => std::env::set_var(ra_providers::openrouter::LOCAL_URL_VAR, value),
            None => std::env::remove_var(ra_providers::openrouter::LOCAL_URL_VAR),
        }
    }
    drop(guard);
    let text = format!("{error:#}");
    assert!(text.contains("the provider is unreachable"), "{text}");
    assert!(text.contains(&url), "{text}");
}

#[tokio::test]
async fn a_local_model_pings_a_server_on_the_loopback() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://127.0.0.1:{}/v1", listener.local_addr().unwrap().port());
    std::thread::spawn(move || {
        if let Ok((mut socket, _)) = listener.accept() {
            let mut buffer = [0u8; 4096];
            let _ = socket.read(&mut buffer);
            let body =
                r#"{"choices":[{"message":{"content":"pong","role":"assistant"}}],"usage":{"prompt_tokens":7,"completion_tokens":1}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes());
        }
    });
    let guard = crate::secret::ENV_LOCK.lock();
    let before = std::env::var(ra_providers::openrouter::LOCAL_URL_VAR).ok();
    // SAFETY: the test holds the env lock; no parallel test reads the variable
    unsafe { std::env::set_var(ra_providers::openrouter::LOCAL_URL_VAR, &url) };
    let worker = Worker::new("local/qwen3");
    let result = run("/ping", &store().await, &worker).await;
    // SAFETY: still under the env lock
    unsafe {
        match before {
            Some(value) => std::env::set_var(ra_providers::openrouter::LOCAL_URL_VAR, value),
            None => std::env::remove_var(ra_providers::openrouter::LOCAL_URL_VAR),
        }
    }
    drop(guard);
    let (out, _) = result.unwrap();
    assert_eq!(out[0], "qwen3 says: pong");
    assert!(out[1].contains("in 7 out 1"), "{out:?}");
    assert_eq!(worker.route(), crate::agent::Route::Local);
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
    let (out, _) = run("/add task Rhipper parent 1", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["#2 Rhipper"]);
    let (out, _) = run("/show", &store, &worker()).await.unwrap();
    assert!(out.iter().any(|l| l.contains("#2 Rhipper")));
}

#[tokio::test]
async fn usage_prints_the_ledger_by_card_summed_up_the_tree() {
    let store = store().await;
    run("/add milestone M parent 1", &store, &worker()).await.unwrap();
    run("/add task A parent 2", &store, &worker()).await.unwrap();
    run("/add task B parent 2", &store, &worker()).await.unwrap();
    run("/add task C parent 1", &store, &worker()).await.unwrap();
    let ledger = ra_usage::UsageStore::in_memory().await.unwrap();
    let entry = |node: Option<i64>, cost: f64| ra_usage::Entry {
        model: "m".to_owned(),
        role: None,
        node,
        prompt_tokens: 100,
        completion_tokens: 10,
        cached_tokens: 0,
        cost_usd: Some(cost),
        cost_computed: false,
        turns: 2,
        read_in: 60,
    };
    // two calls on A, one on C, one on a card the roadmap lost, one outside any card
    for e in [entry(Some(3), 0.10), entry(Some(3), 0.15), entry(Some(5), 0.10), entry(Some(99), 0.05), entry(None, 0.01)] {
        ledger.record(&e).await.unwrap();
    }
    // a worker without a ledger has nothing to print
    assert!(run("/usage", &store, &worker()).await.is_err());
    let worker = worker().with_ledger(ledger);
    let (out, _) = run("/usage", &store, &worker).await.unwrap();
    assert_eq!(
        out,
        vec![
            "#1 Rhipper · 4 calls · 8 turns · 400 in · 40 out · 0 cached · 240 read-in · $0.4000",
            "  #2 M · 2 calls · 4 turns · 200 in · 20 out · 0 cached · 120 read-in · $0.2500",
            "    #3 A · 2 calls · 4 turns · 200 in · 20 out · 0 cached · 120 read-in · $0.2500",
            "  #5 C · 1 calls · 2 turns · 100 in · 10 out · 0 cached · 60 read-in · $0.1000",
            "outside any card · 1 calls · 2 turns · 100 in · 10 out · 0 cached · 60 read-in · $0.0100",
        ]
    );
    // the subtree of the milestone: B has no calls and is left out, as the strays are
    let (out, _) = run("/usage 2", &store, &worker).await.unwrap();
    assert_eq!(
        out,
        vec![
            "  #2 M · 2 calls · 4 turns · 200 in · 20 out · 0 cached · 120 read-in · $0.2500",
            "    #3 A · 2 calls · 4 turns · 200 in · 20 out · 0 cached · 120 read-in · $0.2500",
        ]
    );
    let (out, _) = run("/usage 4", &store, &worker).await.unwrap();
    assert_eq!(out, vec!["the ledger is empty: no completion has run yet"]);
    assert!(run("/usage 42", &store, &worker).await.is_err());
}

#[tokio::test]
async fn usage_by_card_sums_every_run_of_a_card() {
    let ledger = ra_usage::UsageStore::in_memory().await.unwrap();
    let entry = |node: i64, turns: u64, cost: Option<f64>| ra_usage::Entry {
        model: "m".to_owned(),
        role: None,
        node: Some(node),
        prompt_tokens: 1_000,
        completion_tokens: 100,
        cached_tokens: 0,
        cost_usd: cost,
        cost_computed: false,
        turns,
        read_in: 300,
    };
    // two runs on card 3, one on card 4 whose model reports no cost
    for e in [entry(3, 6, Some(0.30)), entry(3, 4, Some(0.20)), entry(4, 5, None)] {
        ledger.record(&e).await.unwrap();
    }
    // a worker without a ledger reads nothing; with one, the costliest card first
    assert!(worker().by_card().await.is_none());
    let cards = worker().with_ledger(ledger).by_card().await.unwrap();
    assert_eq!(
        cards.iter().map(|c| (c.node, c.totals.runs, c.totals.turns, c.totals.cost_usd, c.tokens_per_turn())).collect::<Vec<_>>(),
        [(3, 2, 10, Some(0.5), 220), (4, 1, 5, None, 220)]
    );
}

#[tokio::test]
async fn adds_a_task_under_its_parent() {
    let store = store().await;
    let (out, _) = run("/add task Write the decoder parent 1", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["#2 Write the decoder"]);
}

#[tokio::test]
async fn a_task_needs_a_parent() {
    let store = store().await;
    let error = run("/add task Orphan", &store, &worker()).await.unwrap_err();
    assert_eq!(format!("{error:#}"), "a node below a project needs parent <id>");
}

#[tokio::test]
async fn a_project_takes_no_parent() {
    let store = store().await;
    let error = run("/add project Root parent 7", &store, &worker()).await.unwrap_err();
    assert_eq!(format!("{error:#}"), "a project has no parent");
}

#[tokio::test]
async fn sets_states() {
    let store = store().await;
    run("/add task T parent 1", &store, &worker()).await.unwrap();
    let (out, _) = run("/set 2 in_progress", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["#2 is now in_progress"]);
    let error = run("/set 2 nonsense", &store, &worker()).await.unwrap_err();
    assert!(format!("{error:#}").starts_with("the state is defined, ready, "), "{error:#}");
}

#[tokio::test]
async fn assign_sets_and_clears_the_model_of_a_node() {
    let store = store().await;
    run("/add task T parent 1", &store, &worker()).await.unwrap();
    let (out, _) = run("/assign 2 claude-haiku-4-5", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["#2 runs on claude-haiku-4-5 through Claude Code"]);
    assert_eq!(store.read(2).await.unwrap().assignee.unwrap().model, "claude-haiku-4-5");
    let (out, _) = run("/assign 2 none", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["#2 runs on the badge model"]);
    assert!(store.read(2).await.unwrap().assignee.is_none());
}

#[tokio::test]
async fn run_honors_the_model_of_the_node() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task T parent 1", &store, &worker()).await.unwrap();
    run("/assign 2 z-ai/glm-5.3-flash", &store, &worker()).await.unwrap();
    let provider = Scripted::new(vec![says("done", Vec::new())]);
    // the badge is another OpenRouter model, so the scripted provider serves both; the node's
    // own model is what the request names
    let worker = Worker::new("deepseek/deepseek-v4-flash").with_provider(provider.clone());
    let (out, _) = run("/run 2", &store, &worker).await.unwrap();
    assert!(out[0].contains("on glm-5.3-flash"), "{out:?}");
    assert_eq!(provider.seen.lock()[0].model, "z-ai/glm-5.3-flash");
}

#[tokio::test]
async fn files_names_the_paths_and_the_prompt_lists_them() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task T parent 1", &store, &worker()).await.unwrap();
    let (out, _) = run("/files 2 crates/a/src/lib.rs crates\\b\\src\\main.rs", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["#2 names 2 files: crates/a/src/lib.rs crates/b/src/main.rs"]);
    let (out, _) = run("/show 2", &store, &worker()).await.unwrap();
    assert!(out.iter().any(|l| l == "    files: crates/a/src/lib.rs crates/b/src/main.rs"), "{out:?}");

    let provider = Scripted::new(vec![says("done", Vec::new())]);
    let runner = worker().with_provider(provider.clone());
    run("/run 2", &store, &runner).await.unwrap();
    let seen = provider.seen.lock();
    let prompt = seen[0].messages.iter().map(|m| m.content.as_str()).collect::<Vec<_>>().join("\n");
    let at = prompt.find("Files:").unwrap_or_else(|| panic!("no Files: in {prompt}"));
    let after = &prompt[at..];
    assert!(after.contains("crates/a/src/lib.rs") && after.contains("crates/b/src/main.rs"), "{prompt}");
    drop(seen);

    let (out, _) = run("/files 2", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["#2 names no files"]);
    assert!(store.read(2).await.unwrap().files.is_empty());
}

#[tokio::test]
async fn a_task_without_files_takes_its_sections_and_the_preview_outlines_them() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "//! The library.\n\npub fn inherited_outline() {}\n").unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add section S parent 1", &store, &worker()).await.unwrap();
    run("/files 2 src/lib.rs", &store, &worker()).await.unwrap();
    run("/add task T parent 2", &store, &worker()).await.unwrap();
    let preview = crate::agent::context_preview(&store, 3).await.unwrap();
    // the brief says whose files they are, and the outline of the section's file follows
    assert!(preview.contains("Files of the card, where the change lands, from #2 S:\n  src/lib.rs\n"), "{preview}");
    let at = preview.find("Files:").unwrap_or_else(|| panic!("no Files: in {preview}"));
    assert!(preview[at..].contains("inherited_outline"), "{preview}");
    // the lint asks nothing of the card: the section's files stand for it
    let faults = ra_roadmap::tree::lint(&store.all().await.unwrap());
    assert!(!faults.iter().any(|l| l.contains("no files")), "{faults:?}");
}

#[tokio::test]
async fn context_prints_the_preview_of_a_card_folded_under_a_header_and_the_project_without_an_id() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn outlined() {}\n").unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task T parent 1", &store, &worker()).await.unwrap();
    run("/files 2 src/lib.rs", &store, &worker()).await.unwrap();
    let (out, outcome) = run("/context 2", &store, &worker()).await.unwrap();
    assert!(matches!(outcome, Outcome::Fold), "{outcome:?}");
    assert_eq!(out[0], format!("context #2 · {} lines", out.len() - 1));
    assert!(out.iter().any(|l| l.contains("src/lib.rs (1 lines)")), "{out:?}");
    assert!(out.iter().any(|l| l.contains("pub fn outlined()")), "{out:?}");
    // the project context is the file beside the roadmap, and a missing one says so
    let err = run("/context", &store, &worker()).await.unwrap_err();
    assert!(err.to_string().contains(agent::PROJECT_FILE), "{err:#}");
    std::fs::write(dir.path().join(agent::PROJECT_FILE), "# Rhipper\n\nA project.\n").unwrap();
    let (out, outcome) = run("/context", &store, &worker()).await.unwrap();
    assert!(matches!(outcome, Outcome::Fold), "{outcome:?}");
    assert_eq!(out, [format!("{} · 3 lines", agent::PROJECT_FILE), "# Rhipper".into(), String::new(), "A project.".into()]);
    assert!(run("/context 9", &store, &worker()).await.is_err());
}

#[tokio::test]
async fn edit_rewrites_one_field_and_leaves_the_rest() {
    let store = store().await;
    run("/add task T parent 1", &store, &worker()).await.unwrap();
    store.set_notes(2, "a report").await.unwrap();
    store.set_state(2, TaskState::InProgress).await.unwrap();
    store.set_session(2, Some(("sess-1", "abc")), 7).await.unwrap();
    let (out, _) = run("/edit 2 objective do the thing, well.", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["#2 objective rewritten"]);
    let node = store.read(2).await.unwrap();
    assert_eq!(node.objective.as_deref(), Some("do the thing, well."));
    assert_eq!(node.title, "T");
    assert_eq!(node.notes.as_deref(), Some("a report"));
    assert_eq!(node.state, TaskState::InProgress);
    assert_eq!(node.session.as_deref(), Some("sess-1"));

    run("/edit 2 title Edited", &store, &worker()).await.unwrap();
    run("/edit 2 complexity 4", &store, &worker()).await.unwrap();
    run("/edit 2 files a\\b.rs c.rs", &store, &worker()).await.unwrap();
    let (out, _) = run("/edit 2 session clear", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["#2 forgets its session"]);
    let node = store.read(2).await.unwrap();
    assert_eq!(node.title, "Edited");
    assert_eq!(node.complexity, 4);
    assert_eq!(node.files, vec!["a/b.rs", "c.rs"]);
    assert!(node.session.is_none());
    assert_eq!(node.notes.as_deref(), Some("a report"));

    assert!(run("/edit 2 objective", &store, &worker()).await.is_err());
    assert!(run("/edit 2 colour red", &store, &worker()).await.is_err());
    assert!(run("/edit 2 complexity 9", &store, &worker()).await.is_err());
    assert!(run("/edit 2 session off", &store, &worker()).await.is_err());
}

#[tokio::test]
async fn completes_names() {
    assert_eq!(complete("se"), ["set", "seed"]);
    assert_eq!(complete("q"), ["quit"]);
}

#[test]
fn tab_completes_ids_from_titles_and_states() {
    let nodes = ra_roadmap::markdown::parse(
        "- 1 ‹project› Rhipper\n- 25 ‹task› The key store\n- 26 ‹task› Tab completes ids\n- 3 ‹task› Tab filters",
    )
    .unwrap();
    assert_eq!(complete_line("/ru", &nodes), Completion::Line("/run ".into()));
    assert_eq!(complete_line("/run key", &nodes), Completion::Line("/run 25 ".into()));
    assert_eq!(complete_line("show Tab completes", &nodes), Completion::Line("show 26 ".into()));
    assert_eq!(complete_line("/set 2", &nodes), Completion::Choices(vec!["#25 The key store".into(), "#26 Tab completes ids".into()]));
    assert_eq!(complete_line("/run tab", &nodes), Completion::Choices(vec!["#26 Tab completes ids".into(), "#3 Tab filters".into()]));
    assert_eq!(complete_line("/run nothing", &nodes), Completion::None);
    let Completion::Choices(all) = complete_line("/set 25 ", &nodes) else { panic!("every state") };
    assert_eq!(all.len(), 12);
    assert_eq!(complete_line("/set 25 acc", &nodes), Completion::Line("/set 25 accepted ".into()));
    assert_eq!(complete_line("/set 25 re", &nodes), Completion::Choices(vec!["ready".into(), "review".into()]));
    // a command without an id argument, and the second word of one that takes no state
    assert_eq!(complete_line("/note key", &nodes), Completion::None);
    assert_eq!(complete_line("/run 25 d", &nodes), Completion::None);
}

#[tokio::test]
async fn show_indents_by_depth_and_names_a_missing_node() {
    let store = store().await;
    run("/add task T parent 1", &store, &worker()).await.unwrap();
    let (out, _) = run("/show", &store, &worker()).await.unwrap();
    assert!(out[0].starts_with("#1 "), "{out:?}");
    assert!(out[1].starts_with("  #2 T"), "{out:?}");
    let error = run("/show 9", &store, &worker()).await.unwrap_err();
    assert_eq!(format!("{error:#}"), "no node 9");
}

#[tokio::test]
async fn add_names_the_kinds_on_a_bad_one() {
    let store = store().await;
    let error = run("/add epic T parent 1", &store, &worker()).await.unwrap_err();
    assert_eq!(format!("{error:#}"), "the kind is project, milestone, section, task or subtask");
}

#[tokio::test]
async fn seed_fills_an_empty_roadmap_and_refuses_a_second_time() {
    let store = store().await;
    let (out, _) = run("/seed", &store, &worker()).await.unwrap();
    assert!(out[0].contains("planted"), "{}", out[0]);
    let error = run("/seed", &store, &worker()).await.unwrap_err();
    assert!(format!("{error:#}").contains("already holds nodes"), "{error:#}");
    let (out, _) = run("/show", &store, &worker()).await.unwrap();
    assert!(out.iter().any(|l| l.contains("ping")), "the seed holds the ping task: {out:?}");
}

/// A provider that plays a script: each completion pops the next answer. It records the requests
/// it saw, so a test can read what the loop sent. The plan tests share it.
pub(crate) struct Scripted {
    answers: parking_lot::Mutex<Vec<ra_providers::Completion>>,
    pub(crate) seen: parking_lot::Mutex<Vec<ra_providers::CompletionRequest>>,
}

impl Scripted {
    pub(crate) fn new(answers: Vec<ra_providers::Completion>) -> std::sync::Arc<Self> {
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

#[tokio::test]
async fn gambit_commands_add_change_and_list_them_and_priority_marks_a_card() {
    let store = store().await;
    run("/add task Ledger parent 1", &store, &worker()).await.unwrap();
    let (out, _) = run("/gambits", &store, &worker()).await.unwrap();
    assert!(out[0].starts_with("no gambits yet"), "{out:?}");
    let (out, _) = run("/gambit add Ship the ledger first w5", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["g1 Ship the ledger first {w5} [active]"]);
    // without a trailing w<n> the weight is 3 and the words all go in the text
    let (out, _) = run("/gambit add Keep it local", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["g2 Keep it local {w3} [active]"]);
    let (out, _) = run("/gambit g1 focus ledger usage", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["g1 focuses on ledger usage"]);
    let (out, _) = run("/gambit 2 park", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["g2 is now parked"]);
    let (out, _) = run("/gambit 2 weight 4", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["g2 weighs 4"]);
    let (out, _) = run("/gambits", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["g1 Ship the ledger first {w5} [active]", "    focus: ledger usage", "g2 Keep it local {w4} [parked]"]);
    let (out, _) = run("/gambit 2 activate", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["g2 is now active"]);
    let (out, _) = run("/gambit 1 done", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["g1 is now done"]);
    let gambits = store.gambits().await.unwrap();
    assert_eq!(gambits[0].state, GambitState::Done);
    assert_eq!(gambits[1].weight, 4);
    // the errors name what is wrong
    let error = run("/gambit 9 park", &store, &worker()).await.unwrap_err();
    assert!(format!("{error:#}").contains("no gambit g9"), "{error:#}");
    let error = run("/gambit 1 weight 7", &store, &worker()).await.unwrap_err();
    assert!(format!("{error:#}").contains("1 to 5"), "{error:#}");
    let error = run("/gambit 1 shelve", &store, &worker()).await.unwrap_err();
    assert!(format!("{error:#}").starts_with("usage: gambit"), "{error:#}");

    let (out, _) = run("/priority 2 5", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["#2 has priority 5"]);
    assert_eq!(store.read(2).await.unwrap().priority, Some(5));
    let error = run("/priority 2 6", &store, &worker()).await.unwrap_err();
    assert!(format!("{error:#}").contains("1 to 5"), "{error:#}");
    let (out, _) = run("/priority 2 none", &store, &worker()).await.unwrap();
    assert_eq!(out[0], "#2 has no priority: the file and the gambits order it");
    assert_eq!(store.read(2).await.unwrap().priority, None);
}

pub(crate) fn says(text: &str, calls: Vec<ra_providers::ToolCall>) -> ra_providers::Completion {
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

#[tokio::test]
async fn run_on_a_bare_model_loops_through_the_tools_and_lands_the_answer() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Write the greeting parent 1", &store, &worker()).await.unwrap();
    let write = ra_providers::ToolCall {
        id: "call_1".into(),
        name: "write_file".into(),
        arguments: r#"{"path":"hello.txt","content":"hello from the worker\n"}"#.into(),
    };
    let read = ra_providers::ToolCall { id: "call_2".into(), name: "read_file".into(), arguments: r#"{"path":"hello.txt"}"#.into() };
    cargo_project(dir.path(), true);
    // the three turns of the worker, then the reviewer's verdict on the implemented card
    let provider = Scripted::new(vec![
        says("", vec![write]),
        says("checking", vec![read]),
        says("wrote hello.txt and read it back", vec![]),
        says("accepted", vec![]),
    ]);
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(provider.clone());

    let (out, _) = run("/run 2", &store, &worker).await.unwrap();
    // no cap by default: the line names the turns used alone
    assert!(out[0].contains("turns 3, 2 tool call(s)"), "{out:?}");
    assert!(out[0].contains("$0.03"), "{out:?}");
    assert_eq!(std::fs::read_to_string(dir.path().join("hello.txt")).unwrap(), "hello from the worker\n");
    let node = store.read(2).await.unwrap();
    assert_eq!(node.state, ra_roadmap::model::TaskState::Implemented);
    assert_eq!(node.notes.as_deref(), Some("wrote hello.txt and read it back"));
    assert_eq!(worker.tally().calls, 4);

    // the loop sent the tools, the workspace, and the tool results in order
    let seen = provider.seen.lock();
    assert_eq!(seen.len(), 4);
    assert_eq!(seen[0].tools.len(), 6);
    assert_eq!(seen[0].workspace.as_deref().map(|p| p.canonicalize().unwrap()), Some(dir.path().canonicalize().unwrap()));
    let last = &seen[2].messages;
    assert_eq!(last[last.len() - 1].role, "tool");
    assert!(last[last.len() - 1].content.starts_with("    1│hello from the worker"), "{}", last[last.len() - 1].content);
    assert_eq!(last[last.len() - 3].tool_call_id.as_deref(), Some("call_1"));
}

/// A turn that read and wrote `tokens` in all, past or under the ledger's ceiling.
fn says_at(tokens: u64, text: &str, calls: Vec<ra_providers::ToolCall>) -> ra_providers::Completion {
    let mut answer = says(text, calls);
    answer.usage.prompt_tokens = tokens - answer.usage.completion_tokens;
    answer.turn_usage = vec![ra_providers::TurnUsage::of(&answer.usage)];
    answer
}

#[tokio::test]
async fn a_run_whose_mean_turn_passes_the_ceiling_is_noted_heavy_with_its_biggest_tool_result() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Read it all parent 1", &store, &worker()).await.unwrap();
    cargo_project(dir.path(), true);
    std::fs::write(dir.path().join("big.txt"), "a line of the big file\n".repeat(50)).unwrap();
    let big = ra_providers::ToolCall { id: "call_1".into(), name: "read_file".into(), arguments: r#"{"path":"big.txt"}"#.into() };
    let small = ra_providers::ToolCall { id: "call_2".into(), name: "list_dir".into(), arguments: r#"{"path":"."}"#.into() };
    let ceiling = crate::retro::TURN_CEILING;
    // two turns at 60,000 tokens and one at 10: the mean passes the ceiling
    let provider = Scripted::new(vec![says_at(60_000, "", vec![big, small]), says_at(60_000, "", vec![]), says("accepted", vec![])]);
    let heavy = Worker::new("z-ai/glm-5.3-flash").with_provider(provider);
    let (out, _) = run("/run 2", &store, &heavy).await.unwrap();
    let heavy = out.iter().find(|l| l.contains("[heavy]")).unwrap_or_else(|| panic!("{out:?}"));
    let expected = format!("[heavy] #2 60000 tokens/turn over the ceiling of {ceiling}; biggest tool result: read_file big.txt (");
    assert!(heavy.starts_with(&format!("  ⚠ {expected}")), "{heavy}");
    // the line is kept under the card for the next walk, and left out of the next worker's prompt
    let notes = crate::retro::recent(dir.path(), 5);
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].node, Some(2));
    assert!(notes[0].text.starts_with(&expected), "{}", notes[0].text);
    assert!(!crate::retro::machine_lines(dir.path()).contains("[heavy]"));
    // a run under the ceiling writes nothing
    run("/add task Light parent 1", &store, &worker()).await.unwrap();
    let light = Worker::new("z-ai/glm-5.3-flash").with_provider(Scripted::new(vec![says("done", vec![]), says("accepted", vec![])]));
    let (out, _) = run("/run 3", &store, &light).await.unwrap();
    assert!(!out.iter().any(|l| l.contains("[heavy]")), "{out:?}");
    assert_eq!(crate::retro::recent(dir.path(), 5).len(), 1);
}

#[tokio::test]
async fn a_project_turn_cap_ends_the_run_with_a_handoff_and_the_walk_takes_the_card_up_fresh() {
    let dir = tempfile::tempdir().unwrap();
    cargo_project(dir.path(), true);
    // a repository, so the handoff can name what the run changed
    git_ok(dir.path(), &["init", "-q"]);
    std::fs::write(dir.path().join(crate::router::ROUTES_FILE), "[worker]\nturns = 2\n").unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    let write = ra_providers::ToolCall {
        id: "call_1".into(),
        name: "write_file".into(),
        arguments: r#"{"path":"hello.txt","content":"hello from the worker\n"}"#.into(),
    };
    let read = ra_providers::ToolCall { id: "call_2".into(), name: "read_file".into(), arguments: r#"{"path":"hello.txt"}"#.into() };
    // three turns of work: the cap ends the first run after two, the third lands in the second,
    // which the reviewer then accepts
    let script = Scripted::new(vec![
        says("", vec![write]),
        says("checking", vec![read]),
        says("wrote hello.txt and read it back", vec![]),
        says("accepted", vec![]),
    ]);
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(script.clone());

    let (out, _) = run("/run 2", &store, &runner).await.unwrap();
    assert!(out[0].starts_with("↻ #2 Probe continues on glm-5.3-flash · turns 2/2, 2 tool call(s)"), "{out:?}");
    assert!(out[0].ends_with("the turn cap of 2 ended the run; the card continues in a fresh session"), "{out:?}");
    let node = store.read(2).await.unwrap();
    assert_eq!(node.state, ra_roadmap::model::TaskState::InProgress);
    assert_eq!(node.session, None, "the next run starts fresh");
    // the handoff says how it ended, what was done and what is left
    let task = ra_context::TaskContext::open(dir.path(), 2);
    assert_eq!(task.handoffs.len(), 1);
    let last = &task.handoffs[0];
    assert_eq!(last.outcome, "continues");
    assert!(last.check.starts_with("no check: the turn cap of 2 ended the run"), "{}", last.check);
    assert!(last.report.contains("changed: hello.txt"), "{}", last.report);
    assert!(last.report.contains("left: the check did not run; the acceptance stands:"), "{}", last.report);
    assert!(last.report.contains("checking\ncalls: write_file hello.txt · read_file hello.txt"), "{}", last.report);

    // the walk picks the card up again, and the second run finishes it
    let out = crate::agent::run_all(&store, &runner, false).await.unwrap();
    assert!(out[0].starts_with("✓ #2 Probe implemented on glm-5.3-flash · turns 1/2, 0 tool call(s)"), "{out:?}");
    assert_eq!(out.last().unwrap(), "ran 1 card(s) · no card is ready");
    // the walk lands a card the reviewer accepted: it ends accepted, not waiting
    let node = store.read(2).await.unwrap();
    assert_eq!(node.state, ra_roadmap::model::TaskState::Accepted);
    assert_eq!(node.notes.as_deref(), Some("wrote hello.txt and read it back"));
    assert_eq!(ra_context::TaskContext::open(dir.path(), 2).handoffs.len(), 2);
    // the second run opened on a small context: no tool result of the first run, only its handoff
    let seen = script.seen.lock();
    assert_eq!(seen.len(), 4);
    assert!(seen[2].resume.is_none());
    assert!(seen[2].messages.iter().all(|m| m.role != "tool"), "{:?}", seen[2].messages);
    let prompt = &seen[2].messages.last().unwrap().content;
    assert!(prompt.contains("the turn cap of 2 ended the run"), "{prompt}");
}

/// A harness whose script may end a run out of turns or of context: each completion pops the
/// next outcome, an answer or the error the harness reports with the run's usage, and keeps the
/// request it was asked.
struct CappedHarness {
    script: parking_lot::Mutex<Vec<Result<ra_providers::Completion, ra_providers::ProviderError>>>,
    requests: parking_lot::Mutex<Vec<ra_providers::CompletionRequest>>,
}

impl CappedHarness {
    fn new(script: Vec<Result<ra_providers::Completion, ra_providers::ProviderError>>) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self { script: parking_lot::Mutex::new(script), requests: parking_lot::Mutex::new(Vec::new()) })
    }
}

impl ra_providers::ModelProvider for CappedHarness {
    fn complete(&self, request: ra_providers::CompletionRequest) -> ra_providers::CompletionFuture<'_> {
        self.requests.lock().push(request);
        let next = self.script.lock().pop();
        Box::pin(async move { next.unwrap_or_else(|| Err(ra_providers::ProviderError::Other("the script ran out".into()))) })
    }

    fn has_own_tools(&self) -> bool {
        true
    }
}

#[tokio::test]
async fn a_harness_run_the_turn_cap_ends_books_its_usage_and_the_card_sums_its_sessions() {
    let dir = tempfile::tempdir().unwrap();
    cargo_project(dir.path(), true);
    git_ok(dir.path(), &["init", "-q"]);
    std::fs::write(dir.path().join(crate::router::ROUTES_FILE), "[worker]\nturns = 2\n").unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    // the capped run reported 1000 in, 52 out and $1.25 before its turns ran out
    let mut capped = says("half way", vec![]);
    capped.turns = 2;
    capped.usage =
        ra_providers::Usage { prompt_tokens: 1000, completion_tokens: 52, cached_tokens: 900, cost_usd: Some(1.25), cost_computed: false };
    capped.turn_usage = vec![
        ra_providers::TurnUsage { input_tokens: 400, output_tokens: 2 },
        ra_providers::TurnUsage { input_tokens: 600, output_tokens: 50 },
    ];
    // it never wrote: the read-in is the whole input, two files read whole
    capped.read_in = 1000;
    capped.whole_reads = 2;
    let script = vec![Ok(says("done", vec![])), Err(ra_providers::ProviderError::TurnsOut(Box::new(capped)))];
    let ledger = ra_usage::UsageStore::in_memory().await.unwrap();
    let runner = Worker::new("claude-opus-5-5").with_provider(CappedHarness::new(script)).with_ledger(ledger);

    let (out, _) = run("/run 2", &store, &runner).await.unwrap();
    assert!(
        out[0].starts_with(
            "↻ #2 Probe continues on claude-opus-5-5 · turns 2/2, 0 tool call(s) · in 1000 (90% cached) out 52 · read-in 1k, 2 files whole · avg"
        ),
        "{out:?}"
    );
    assert!(out[0].contains("tokens a turn, largest 650 · $1.2500 · the turn cap of 2 ended the run"), "{out:?}");
    // the ledger has the session as a finished run's, under the worker with no role
    let calls = runner.calls_of(Some(2)).await.unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!((calls[0].prompt_tokens, calls[0].completion_tokens, calls[0].cost_usd, calls[0].turns), (1000, 52, Some(1.25), 2));
    assert_eq!((calls[0].role.as_deref(), calls[0].read_in, calls[0].cached_tokens), (None, 1000, 900));
    // the feed's end line carries the same
    let feed = crate::pair::last_end(dir.path()).unwrap();
    assert!(feed.contains("in 1000 (90% cached) out 52") && feed.contains("$1.2500"), "{feed}");
    assert!(ra_context::TaskContext::open(dir.path(), 2).handoffs[0].report.contains("half way"));

    // the next session finishes the card, and the card's cost is both sessions together
    let out = crate::agent::run_all(&store, &runner, false).await.unwrap();
    assert!(out[0].starts_with("✓ #2 Probe implemented on claude-opus-5-5"), "{out:?}");
    let by_node = runner.by_node().await.unwrap();
    let card = by_node.iter().find(|(node, _)| *node == Some(2)).map(|(_, totals)| totals).unwrap();
    assert_eq!((card.calls, card.prompt_tokens, card.completion_tokens, card.turns), (2, 1100, 62, 3));
    assert!((card.cost_usd.unwrap() - 1.26).abs() < 1e-9, "{:?}", card.cost_usd);
}

#[tokio::test]
async fn a_harness_run_the_context_cap_ends_continues_fresh_and_the_end_line_names_the_cap_and_the_turn() {
    let dir = tempfile::tempdir().unwrap();
    cargo_project(dir.path(), true);
    git_ok(dir.path(), &["init", "-q"]);
    // the project names a context cap of 120k and no turn cap; without the line no cap stands
    std::fs::write(dir.path().join("rhipper.toml"), "[worker]\ncontext = 120000\n").unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    // the stream showed five turns, the last of 131k input, before the harness stopped the run
    let mut capped = says("editing the store", vec![]);
    capped.turns = 5;
    capped.usage = ra_providers::Usage {
        prompt_tokens: 400_000,
        completion_tokens: 500,
        cached_tokens: 390_000,
        cost_usd: None,
        cost_computed: false,
    };
    capped.turn_usage = (1..=5).map(|n| ra_providers::TurnUsage { input_tokens: n * 26_200, output_tokens: 100 }).collect();
    capped.read_in = 400_000;
    // it read the manifest whole and the lib by two touching ranges
    capped.reads = ["src/lib.rs:1-1", "Cargo.toml", "src/lib.rs:2-2"].map(str::to_owned).into();
    let script = vec![
        Ok(says("done", vec![])),
        Err(ra_providers::ProviderError::ContextOut { completion: Box::new(capped), cap: 120_000, size: 131_000 }),
    ];
    let harness = CappedHarness::new(script);
    let runner =
        Worker::new("claude-opus-5-5").with_provider(harness.clone()).with_ledger(ra_usage::UsageStore::in_memory().await.unwrap());

    let (out, _) = run("/run 2", &store, &runner).await.unwrap();
    assert!(
        out[0].starts_with(
            "↻ #2 Probe continues on claude-opus-5-5 · turns 5, 0 tool call(s) · in 400000 (97% cached) out 500 · read-in 400k, 0 files whole · avg"
        ),
        "{out:?}"
    );
    assert!(
        out[0].ends_with("the context cap of 120k tokens (a turn of 131k) ended the run; the card continues in a fresh session"),
        "{out:?}"
    );
    assert_eq!(harness.requests.lock().last().unwrap().max_context, Some(120_000), "the project's cap went with the request");
    let node = store.read(2).await.unwrap();
    assert_eq!(node.state, ra_roadmap::model::TaskState::InProgress);
    assert_eq!(node.session, None, "the next run starts fresh");
    let task = ra_context::TaskContext::open(dir.path(), 2);
    assert_eq!(task.handoffs.len(), 1);
    assert_eq!(task.handoffs[0].outcome, "continues");
    assert!(
        task.handoffs[0].check.starts_with("no check: the context cap of 120k tokens (a turn of 131k) ended the run"),
        "{}",
        task.handoffs[0].check
    );
    assert!(
        task.handoffs[0].report.starts_with("the context cap of 120k tokens (a turn of 131k) ended the run after 5 turn(s)"),
        "{}",
        task.handoffs[0].report
    );
    assert!(task.handoffs[0].report.ends_with("editing the store"), "{}", task.handoffs[0].report);
    // the handoff names the read set, and the fresh session's prompt opens with its regions
    assert_eq!(task.handoffs[0].read, ["src/lib.rs:1-2", "Cargo.toml"]);
    let text = std::fs::read_to_string(ra_context::TaskContext::path(dir.path(), 2)).unwrap();
    assert!(text.contains("read: src/lib.rs:1-2 Cargo.toml\n"), "{text}");
    // the session is booked, and the walk finishes the card in a fresh one
    let calls = runner.calls_of(Some(2)).await.unwrap();
    assert_eq!((calls.len(), calls[0].prompt_tokens, calls[0].turns), (1, 400_000, 5));
    let out = crate::agent::run_all(&store, &runner, false).await.unwrap();
    assert!(out[0].starts_with("✓ #2 Probe implemented on claude-opus-5-5"), "{out:?}");
    assert!(harness.requests.lock().last().unwrap().resume.is_none());
    // the second request is the fresh session's; a review may follow it
    let opened = harness.requests.lock()[1].messages.last().unwrap().content.clone();
    assert!(opened.starts_with(ra_context::REGIONS_HEADING), "{opened}");
    assert!(opened.contains("src/lib.rs lines 1-2 (of 2)\n") && opened.contains("     1  #[test]\n"), "{opened}");
    assert!(opened.contains("Cargo.toml lines 1-6 (of 6)\n      1-5     [package]\n      6-6     [workspace]\n"), "{opened}");
    assert!(opened.find("Cargo.toml lines").unwrap() < opened.find("Task #2").unwrap(), "{opened}");
    // the cap fell at turn 5, while the session was still reading in: the fresh session reads
    // the same files, so it runs without the cap rather than hit it again (runelite #15)
    assert_eq!(harness.requests.lock().last().unwrap().max_context, None, "no cap after a cap hit reading in");
}

#[tokio::test]
async fn a_finished_run_end_line_carries_the_read_in_and_the_files_read_whole_and_the_ledger_keeps_them() {
    let dir = tempfile::tempdir().unwrap();
    cargo_project(dir.path(), true);
    git_ok(dir.path(), &["init", "-q"]);
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    // what the stream of two turns yields when the first reads three files, two whole, and the
    // second edits (the_stream_tallies_the_read_in_through_the_first_edit_and_the_files_read_whole)
    let mut done = says("done: edited `src/lib.rs`", vec![]);
    done.turns = 2;
    done.usage = ra_providers::Usage {
        prompt_tokens: 3208,
        completion_tokens: 130,
        cached_tokens: 3000,
        cost_usd: Some(0.05),
        cost_computed: false,
    };
    done.turn_usage = vec![
        ra_providers::TurnUsage { input_tokens: 1203, output_tokens: 40 },
        ra_providers::TurnUsage { input_tokens: 2005, output_tokens: 90 },
    ];
    done.read_in = 3208;
    done.whole_reads = 2;
    let ledger = ra_usage::UsageStore::in_memory().await.unwrap();
    let runner = Worker::new("claude-opus-5-5").with_provider(CappedHarness::new(vec![Ok(done)])).with_ledger(ledger);
    let (out, _) = run("/run 2", &store, &runner).await.unwrap();
    assert!(
        out[0].starts_with(
            "✓ #2 Probe implemented on claude-opus-5-5 · turns 2, 0 tool call(s) · in 3208 (93% cached) out 130 · read-in 3.2k, 2 files whole · avg"
        ),
        "{out:?}"
    );
    let calls = runner.calls_of(Some(2)).await.unwrap();
    assert_eq!((calls.len(), calls[0].read_in, calls[0].cached_tokens), (1, 3208, 3000));
    let (usage, _) = run("/usage", &store, &runner).await.unwrap();
    assert!(
        usage.iter().any(|line| line.contains("#2 Probe · 1 calls · 2 turns · 3208 in · 130 out · 3000 cached · 3208 read-in")),
        "{usage:?}"
    );
}

/// The one-off that reads Phase S's baseline back: for every run of the workspace's own
/// `var/activity.log`, the reads before the first edit, which is all the log holds of a
/// session's reading-in before this card's end line. Ignored: it reads the machine's state.
#[test]
#[ignore = "a one-off over the workspace's own activity log"]
fn phase_s_baseline_counts_the_reads_before_the_first_edit_of_each_logged_run() {
    use crate::logs::{RUN_END, RUN_START};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let log = std::fs::read_to_string(root.join(crate::ui::ACTIVITY_FILE)).unwrap_or_default();
    let (mut runs, mut reads, mut whole) = (0u32, 0u32, 0u32);
    let mut open: Option<(String, u32, u32, bool)> = None;
    for line in log.lines() {
        if let Some(head) = line.strip_prefix(RUN_START) {
            open = Some((head.split(" · ").next().unwrap_or(head).to_owned(), 0, 0, false));
        } else if line.starts_with(RUN_END) {
            if let Some((card, before, all, _)) = open.take() {
                println!("{card}: {before} reads before the first edit, {all} in all");
                runs += 1;
                reads += before;
                whole += all;
            }
        } else if let Some((_, before, all, edited)) = &mut open {
            if line.starts_with("Edit ") || line.starts_with("Write ") {
                *edited = true;
            } else if line.starts_with("Read ") {
                *all += 1;
                if !*edited {
                    *before += 1;
                }
            }
        }
    }
    println!("baseline: {runs} runs, {reads} reads before the first edit, {whole} reads in all");
}

#[tokio::test]
async fn a_placeholder_report_is_asked_again_on_its_session_and_a_real_one_is_not() {
    let dir = tempfile::tempdir().unwrap();
    cargo_project(dir.path(), true);
    git_ok(dir.path(), &["init", "-q"]);
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    run("/add task Second parent 1", &store, &worker()).await.unwrap();
    // the first answer only says its check is running; the session, asked again, reports
    let mut placeholder = says("cargo test -p probe is still running in the background", vec![]);
    placeholder.session_id = Some("s-probe".to_owned());
    // popped from the end; the review of each implemented card reads its verdict off the same
    // provider, so one `accepted` follows each report
    let script = vec![
        Ok(says("accepted", vec![])),
        Ok(says("added the probe in `src/lib.rs` and read it back", vec![])),
        Ok(says("accepted", vec![])),
        Ok(says("changed `src/lib.rs`: the probe asserts the flag", vec![])),
        Ok(placeholder),
    ];
    let harness = CappedHarness::new(script);
    let runner =
        Worker::new("claude-opus-5-5").with_provider(harness.clone()).with_ledger(ra_usage::UsageStore::in_memory().await.unwrap());

    let (out, _) = run("/run 2", &store, &runner).await.unwrap();
    assert!(out[0].starts_with("✓ #2 Probe implemented on claude-opus-5-5 · turns 2,"), "{out:?}");
    assert!(out[0].ends_with(" · report asked twice"), "{out:?}");
    {
        let requests = harness.requests.lock();
        let asked: Vec<(Option<&str>, Option<u32>, String)> = requests
            .iter()
            .map(|r| (r.resume.as_deref(), r.max_turns, r.messages.last().unwrap().content.chars().take(100).collect()))
            .collect();
        // the task, the second ask on its session, and the review
        assert_eq!(requests.len(), 3, "{asked:?}");
        assert_eq!(requests[0].resume, None);
        assert_eq!(requests[1].resume.as_deref(), Some("s-probe"));
        assert_eq!(requests[1].max_turns, Some(3));
        let prompt = &requests[1].messages.last().unwrap().content;
        assert!(prompt.contains("named no file and no result") && prompt.contains("git status"), "{prompt}");
    }
    let task = ra_context::TaskContext::open(dir.path(), 2);
    assert_eq!(task.handoffs.len(), 1);
    assert!(task.handoffs[0].report.contains("the probe asserts the flag"), "{}", task.handoffs[0].report);
    assert!(!task.handoffs[0].report.contains("still running"), "{}", task.handoffs[0].report);
    // both asks are on the ledger under the card, as the worker's own calls
    let worker_calls = runner.calls_of(Some(2)).await.unwrap().iter().filter(|call| call.role.is_none()).count();
    assert_eq!(worker_calls, 2);

    // a real report goes to the check and the review as it is: one ask, then the review
    let (out, _) = run("/run 3", &store, &runner).await.unwrap();
    assert!(out[0].starts_with("✓ #3 Second implemented on claude-opus-5-5 · turns 1,"), "{out:?}");
    assert!(!out[0].contains("asked twice"), "{out:?}");
    assert_eq!(harness.requests.lock().len(), 5);
}

#[test]
fn a_report_is_a_placeholder_when_it_names_no_file_and_says_nothing_beyond_the_check() {
    assert!(crate::agent::is_empty_handoff(""));
    assert!(crate::agent::is_empty_handoff("the suite is still running"));
    assert!(crate::agent::is_empty_handoff("running 3 tests\ntest a ... ok\ntest result: ok. 3 passed"));
    assert!(crate::agent::is_empty_handoff("## Report\n\nDone."));
    assert!(!crate::agent::is_empty_handoff("changed `crates/x/src/lib.rs`"));
    assert!(!crate::agent::is_empty_handoff("Nothing to change: the card is already implemented in the store."));
}

/// A one-crate Cargo project in `dir` whose single test passes or fails. The package is named
/// after the directory: the probes of the tests build in one inherited `CARGO_TARGET_DIR`, where
/// cargo hashes a workspace's root package without its path, so two probes of one name share a
/// test binary and a failing one ran a passing one's.
fn cargo_project(dir: &std::path::Path, passing: bool) {
    std::fs::write(
        dir.join("Cargo.toml"),
        format!("[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n", probe_name(dir)),
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/lib.rs"), format!("#[test]\nfn probe() {{ assert!({passing}, \"the probe broke\"); }}\n")).unwrap();
}

/// The package name of the probe in `dir`.
fn probe_name(dir: &std::path::Path) -> String {
    let unique: String = dir.file_name().unwrap_or_default().to_string_lossy().chars().filter(char::is_ascii_alphanumeric).collect();
    format!("probe_{unique}")
}

#[test]
fn the_checks_are_the_backticked_commands_in_order_else_the_workspace_tests() {
    use crate::agent::check_commands;
    assert_eq!(check_commands(Some("`cargo test -p ra-console -q` passes")), ["cargo test -p ra-console -q"]);
    assert_eq!(
        check_commands(Some("run `rc gotchas` then `cargo test -p ra-console` and `cargo clippy`")),
        ["cargo test -p ra-console", "cargo clippy"]
    );
    assert_eq!(
        check_commands(Some("`python tools-src/x.py --only T` passes, then `.\\gw.ps1 :m:task` passes; see `src/lib.rs` and `Node`")),
        ["python tools-src/x.py --only T", ".\\gw.ps1 :m:task"]
    );
    assert_eq!(check_commands(Some("it is done, see `git log`")), ["cargo test --workspace -q"]);
    assert_eq!(check_commands(None), ["cargo test --workspace -q"]);
}

#[tokio::test]
async fn a_run_with_several_checks_stops_at_the_first_failure_and_notes_each() {
    let dir = tempfile::tempdir().unwrap();
    cargo_project(dir.path(), true);
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    run(
        "/edit 2 acceptance `python tools-src/x.py --only T` passes, `.\\gw.ps1 :m:task` passes and `cargo test -q` passes",
        &store,
        &worker(),
    )
    .await
    .unwrap();
    let seen = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    let log = seen.clone();
    let script: ra_tools::Script = std::sync::Arc::new(move |command: &str| {
        log.lock().push(command.to_owned());
        Ok(if command.starts_with(".\\gw.ps1") { (1, "BUILD FAILED\n".to_owned()) } else { (0, "ok\n".to_owned()) })
    });
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(Scripted::new(vec![says("done", vec![])])).with_script(script);
    let (out, _) = run("/run 2", &store, &worker).await.unwrap();
    // the third command never runs
    assert_eq!(*seen.lock(), ["python tools-src/x.py --only T", ".\\gw.ps1 :m:task"]);
    let node = store.read(2).await.unwrap();
    assert_eq!(node.state, ra_roadmap::model::TaskState::NeedsRevision);
    let notes = node.notes.unwrap_or_default();
    assert_eq!(
        notes,
        "check failed: .\\gw.ps1 :m:task\nBUILD FAILED\nexit code 1\n\ncheck passed: python tools-src/x.py --only T · exit code 0\nok\nexit code 0\n\ndone"
    );
    assert!(
        out.iter().any(|l| l.contains("check failed: .\\gw.ps1 :m:task · after 1 of 3 passed · the output is in the notes")),
        "{out:?}"
    );
}

async fn checked_run(passing: bool) -> (Vec<String>, ra_roadmap::model::Node) {
    let dir = tempfile::tempdir().unwrap();
    cargo_project(dir.path(), passing);
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(Scripted::new(vec![says("done", vec![]), says("accepted", vec![])]));
    let (out, _) = run("/run 2", &store, &worker).await.unwrap();
    let node = store.read(2).await.unwrap();
    (out, node)
}

/// The lines of a walk that are runs ending implemented, not the verdicts that follow them.
fn implemented(out: &[String]) -> Vec<&String> {
    out.iter().filter(|l| l.starts_with('✓') && l.contains(" implemented on ")).collect()
}

/// The answers of a walk over cards whose runs each say one of `reports` and are each accepted.
fn accepted_runs(reports: &[&str]) -> Vec<ra_providers::Completion> {
    reports.iter().flat_map(|report| [says(report, vec![]), says("accepted", vec![])]).collect()
}

/// Three tasks under the root, #4 waiting on #3, in a project whose check passes or fails.
async fn three_cards(dir: &std::path::Path, passing: bool) -> Store {
    cargo_project(dir, passing);
    let store = Store::open(&dir.join("rhipper.md"), "Rhipper").await.unwrap();
    for title in ["A", "B", "C"] {
        run(&format!("/add task {title} parent 1"), &store, &worker()).await.unwrap();
    }
    store.depend_on(4, 3).await.unwrap();
    store
}

#[tokio::test]
async fn a_walk_judges_a_landed_cards_saving_on_the_ledger_before_and_after_its_run() {
    use ra_roadmap::store::Card;
    let dir = tempfile::tempdir().unwrap();
    cargo_project(dir.path(), true);
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    // the card claims fewer tokens a turn, on a lesson another project shared
    let card = Card { saving: Some("fewer tokens a turn"), evidence: Some("notes.log:3"), ..Card::new() };
    let id = store.insert_card(Some(1), ra_roadmap::model::NodeKind::Task, "Smaller reads", &card).await.unwrap();
    // the one card the ledger saw before took 1000 tokens a turn; the scripted run's calls carry 110
    let ledger = ra_usage::UsageStore::in_memory().await.unwrap();
    let before = ra_usage::Entry {
        model: "claude-opus-5-5".to_owned(),
        role: None,
        node: Some(99),
        prompt_tokens: 1800,
        completion_tokens: 200,
        cached_tokens: 0,
        cost_usd: None,
        cost_computed: false,
        turns: 2,
        read_in: 0,
    };
    ledger.record(&before).await.unwrap();
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(Scripted::new(accepted_runs(&["did it"]))).with_ledger(ledger);
    let out = crate::agent::run_all(&store, &runner, false).await.unwrap();
    // the run's two turns, its cost from the list prices, against the one card before it
    let held = format!("[saving] #{id} held: 2 turns, 110 tokens/turn, $0.02 vs median 1000; claimed \"fewer tokens a turn\"");
    assert!(out.contains(&held), "{out:?}");
    assert_eq!(out.last().unwrap(), "ran 1 card(s) · no card is ready");
    // the verdict is under the card, shared as a machine note since the lesson held here too
    let notes = crate::retro::recent(dir.path(), 5);
    assert_eq!((notes.len(), notes[0].node, notes[0].machine, notes[0].text.as_str()), (1, Some(id), true, held.as_str()));
    // another project's planner reads it as evidence, tagged; its workers' prompts do not
    let other = tempfile::tempdir().unwrap();
    let evidence = crate::retro::evidence(other.path());
    assert!(evidence.iter().any(|l| l.source.starts_with("notes.log:") && l.text.ends_with(&format!("[machine] {held}"))), "{evidence:?}");
    assert!(!crate::retro::machine_lines(other.path()).contains("[saving]"));
    // a second walk has nothing to judge: the card is done and its claim read
    let out = crate::agent::run_all(&store, &runner, false).await.unwrap();
    assert_eq!(out, vec!["ran 0 card(s) · no card is ready"]);
    assert_eq!(crate::retro::recent(dir.path(), 5).len(), 1);
}

#[tokio::test]
async fn a_saving_line_compares_the_card_to_the_median() {
    use ra_roadmap::store::Card;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    let card = Card { saving: Some("one verdict line instead of rc usage"), ..Card::new() };
    let id = store.insert_card(Some(1), ra_roadmap::model::NodeKind::Task, "Measured", &card).await.unwrap();
    run(&format!("/set {id} implemented"), &store, &worker()).await.unwrap();
    let node = store.read(id).await.unwrap();
    // three landed cards at 300, 1000 and 5000 tokens a turn: the median is 1000
    let ledger = ra_usage::UsageStore::in_memory().await.unwrap();
    let row = |node: i64, turns: u64, prompt_tokens: u64, cost_usd: Option<f64>| ra_usage::Entry {
        model: "claude-opus-5-5".to_owned(),
        role: None,
        node: Some(node),
        prompt_tokens,
        completion_tokens: 0,
        cached_tokens: 0,
        cost_usd,
        cost_computed: false,
        turns,
        read_in: 0,
    };
    for (other, turns, tokens) in [(11, 2, 600), (12, 4, 4000), (13, 1, 5000)] {
        ledger.record(&row(other, turns, tokens, None)).await.unwrap();
    }
    let mark = ledger.last_id().await.unwrap();
    // the card's run: 3 turns at 110 tokens a turn, its cost reported
    ledger.record(&row(id, 3, 330, Some(0.05))).await.unwrap();
    let lines = crate::retro::landed(dir.path(), &node, &ledger, mark).await;
    let held =
        format!("[saving] #{id} held: 3 turns, 110 tokens/turn, $0.05 vs median 1000; claimed \"one verdict line instead of rc usage\"");
    assert_eq!(lines, vec![held.clone()]);
    assert_eq!(crate::retro::recent(dir.path(), 5).iter().map(|n| (n.node, n.text.clone())).collect::<Vec<_>>(), [(Some(id), held)]);
    // a second card, over the median, misses; the first card's rows now count among the twenty
    let card = Card { saving: Some("fewer turns"), ..Card::new() };
    let second = store.insert_card(Some(1), ra_roadmap::model::NodeKind::Task, "Heavy", &card).await.unwrap();
    run(&format!("/set {second} implemented"), &store, &worker()).await.unwrap();
    let node = store.read(second).await.unwrap();
    let mark = ledger.last_id().await.unwrap();
    ledger.record(&row(second, 10, 20_000, None)).await.unwrap();
    let lines = crate::retro::landed(dir.path(), &node, &ledger, mark).await;
    assert_eq!(lines, vec![format!("[saving] #{second} missed: 10 turns, 2000 tokens/turn vs median 300; claimed \"fewer turns\"")]);
    // a card with no other card to measure against gets no verdict
    let empty = ra_usage::UsageStore::in_memory().await.unwrap();
    empty.record(&row(second, 1, 100, None)).await.unwrap();
    assert!(crate::retro::landed(tempfile::tempdir().unwrap().path(), &node, &empty, 0).await.is_empty());
}

#[tokio::test]
async fn run_all_walks_the_ready_cards_in_order_and_a_dependent_after_its_dependency() {
    let dir = tempfile::tempdir().unwrap();
    let store = three_cards(dir.path(), true).await;
    let script = Scripted::new(accepted_runs(&["did A", "did B", "did C"]));
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(script.clone());
    let out = crate::agent::run_all(&store, &runner, false).await.unwrap();
    // #4 is not ready while #3 is open; it runs once #3 is implemented, after #2 and #3
    let ran = implemented(&out);
    assert_eq!(ran.len(), 3, "{out:?}");
    assert!(ran[0].starts_with("✓ #2 A implemented"), "{out:?}");
    assert!(ran[1].starts_with("✓ #3 B implemented"), "{out:?}");
    assert!(ran[2].starts_with("✓ #4 C implemented"), "{out:?}");
    assert_eq!(out.last().unwrap(), "ran 3 card(s) · no card is ready");
    // each card got its own answer in its notes, so each run was a run of its own
    assert_eq!(store.read(2).await.unwrap().notes.as_deref(), Some("did A"));
    assert_eq!(store.read(4).await.unwrap().notes.as_deref(), Some("did C"));
    // each run was judged before its card was offered, and the walk landed each on the
    // verdict: the cards end accepted (a landed card's context is swept with the next save,
    // so the verdict is read from the walk's own lines)
    assert_eq!(script.seen.lock().len(), 6);
    for id in [2, 3, 4] {
        assert!(out.iter().any(|l| l.starts_with(&format!("✓ #{id} ")) && l.contains("accepted by glm-5.3-flash")), "#{id}: {out:?}");
        assert_eq!(store.read(id).await.unwrap().state, ra_roadmap::model::TaskState::Accepted, "#{id}");
    }
}

/// A relaunch that starts nothing and names a pid, for the walks that move to a newer install.
fn relaunched(_dir: &std::path::Path, _root: &std::path::Path) -> anyhow::Result<u32> {
    Ok(4242)
}

#[tokio::test]
async fn a_walk_moves_to_a_newer_installed_rc_after_its_card_and_goes_on_when_the_install_did_not_move() {
    let install = tempfile::tempdir().unwrap();
    let walk = |dir: &std::path::Path, script| {
        Worker::new("z-ai/glm-5.3-flash").with_provider(script).with_install(Some(dir.to_path_buf())).with_relaunch(relaunched)
    };
    // the install is newer than this build: the walk ends after its first card, never inside it
    std::fs::write(install.path().join(crate::hub::INSTALLED_FILE), "99.0.0 1\n").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = three_cards(dir.path(), true).await;
    let out = crate::agent::run_all(&store, &walk(install.path(), Scripted::new(accepted_runs(&["did A"]))), false).await.unwrap();
    assert_eq!(implemented(&out).len(), 1, "{out:?}");
    assert_eq!(out.last().unwrap(), "moved to 99.0.0 · ran 1 card(s) · pid 4242");
    let feed = std::fs::read_to_string(dir.path().join("var/pair.log")).unwrap();
    assert!(feed.contains("walk moved to 99.0.0"), "{feed}");
    // the lock went with the walk: the successor takes it
    assert!(!crate::agent::walk_running(dir.path()));
    // the install is this build: the walk runs every card
    std::fs::write(install.path().join(crate::hub::INSTALLED_FILE), format!("{} 1\n", crate::VERSION)).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = three_cards(dir.path(), true).await;
    let script = Scripted::new(accepted_runs(&["did A", "did B", "did C"]));
    let out = crate::agent::run_all(&store, &walk(install.path(), script), false).await.unwrap();
    assert_eq!(implemented(&out).len(), 3, "{out:?}");
    assert_eq!(out.last().unwrap(), "ran 3 card(s) · no card is ready");
}

#[tokio::test]
async fn a_matching_gambit_pulls_a_later_card_ahead_in_the_walk_and_a_priority_ahead_of_that() {
    let dir = tempfile::tempdir().unwrap();
    let store = three_cards(dir.path(), true).await;
    store.rename(3, "The local route").await.unwrap();
    store.add_gambit("Local first", 3, &["local".to_owned()]).await.unwrap();
    // #3 matches the gambit, so it goes before #2 A; the rest follow in the file's order
    let roadmap = store.load().await.unwrap();
    assert_eq!(crate::agent::walk_order(&roadmap.nodes, &roadmap.gambits, false), vec![3, 2, 4]);
    assert_eq!(ra_roadmap::tree::next_ready(&roadmap.nodes, &roadmap.gambits).map(|n| n.id), Some(3));
    // a priority on #2 puts it ahead of the matching card
    store.set_priority(2, Some(4)).await.unwrap();
    let roadmap = store.load().await.unwrap();
    assert_eq!(crate::agent::walk_order(&roadmap.nodes, &roadmap.gambits, false), vec![2, 3, 4]);
    store.set_priority(2, None).await.unwrap();
    // the walk itself runs in the scored order
    let script = Scripted::new(accepted_runs(&["did B", "did A", "did C"]));
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(script.clone());
    let out = crate::agent::run_all(&store, &runner, false).await.unwrap();
    let ran = implemented(&out);
    assert_eq!(ran.len(), 3, "{out:?}");
    assert!(ran[0].starts_with("✓ #3 The local route implemented"), "{out:?}");
    assert!(ran[1].starts_with("✓ #2 A implemented"), "{out:?}");
    assert_eq!(store.read(3).await.unwrap().notes.as_deref(), Some("did B"));
}

/// The planner's answer for the walk tests: one card under the empty milestone.
const PLANNED_CARD: &str =
    "- Write the greeting {c2}\n  objective: hello.txt holds the greeting\n  acceptance: `cargo test` passes\n  files: src/lib.rs";

/// A project whose check passes, with one empty milestone under the root and nothing ready.
async fn empty_milestone(dir: &std::path::Path) -> Store {
    cargo_project(dir, true);
    let store = Store::open(&dir.join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add milestone M parent 1", &store, &worker()).await.unwrap();
    store.set_details(2, Some("ship the greeting"), None).await.unwrap();
    store
}

#[tokio::test]
async fn a_walk_with_nothing_ready_plans_the_next_milestone_when_a_gambit_says_so_and_runs_its_cards() {
    let dir = tempfile::tempdir().unwrap();
    let store = empty_milestone(dir.path()).await;
    let g1 = store.add_gambit("Keep the roadmap full", 3, &[]).await.unwrap();
    store.set_gambit_plan_next(g1, true).await.unwrap();
    // the planner answers first, then the worker on the card it added, then its reviewer
    let script = Scripted::new(vec![says(PLANNED_CARD, vec![]), says("wrote it", vec![]), says("accepted", vec![])]);
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(script.clone());
    let out = crate::agent::run_all(&store, &runner, false).await.unwrap();
    assert_eq!(out[0], "#3 task Write the greeting {c2}", "{out:?}");
    assert!(out.iter().any(|l| l == "planned #2 M: 1 task added as defined by glm-5.3-flash"), "{out:?}");
    assert!(out.iter().any(|l| l.starts_with("✓ #3 Write the greeting implemented")), "{out:?}");
    assert_eq!(out.last().unwrap(), "ran 1 card(s) · no card is ready and no milestone is left to plan");
    // the card landed defined with its fields and the feed heard of the plan before the run
    let card = store.read(3).await.unwrap();
    assert_eq!((card.parent, card.complexity, card.files.clone()), (Some(2), 2, vec!["src/lib.rs".to_owned()]));
    assert_eq!(card.objective.as_deref(), Some("hello.txt holds the greeting"));
    let feed = crate::pair::recent(dir.path(), 20);
    let planned = feed.iter().position(|l| l == "planned #2 M: 1 task added as defined by glm-5.3-flash").expect(&format!("{feed:?}"));
    let started = feed.iter().position(|l| l.starts_with("start #3")).expect(&format!("{feed:?}"));
    assert!(planned < started, "{feed:?}");
    // the planner read the milestone and was the first of the three calls
    assert_eq!(script.seen.lock().len(), 3);
    assert!(script.seen.lock()[0].messages[1].content.contains("ship the greeting"));
}

#[tokio::test]
async fn a_walk_with_nothing_ready_and_no_plan_next_gambit_ends_as_before() {
    let dir = tempfile::tempdir().unwrap();
    let store = empty_milestone(dir.path()).await;
    // an active gambit without the flag, and a parked one with it, plan nothing
    store.add_gambit("Local first", 3, &[]).await.unwrap();
    let g2 = store.add_gambit("Keep the roadmap full", 3, &[]).await.unwrap();
    store.set_gambit_plan_next(g2, true).await.unwrap();
    store.set_gambit_state(g2, GambitState::Parked).await.unwrap();
    let script = Scripted::new(vec![says(PLANNED_CARD, vec![])]);
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(script.clone());
    let out = crate::agent::run_all(&store, &runner, false).await.unwrap();
    assert_eq!(out, vec!["ran 0 card(s) · no card is ready"]);
    assert_eq!(store.all().await.unwrap().len(), 2);
    assert!(script.seen.lock().is_empty());
}

/// One git command in `dir`, which must succeed; the identity is given so a commit needs none
/// configured on the machine.
fn git_ok(dir: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(["-c", "user.name=test", "-c", "user.email=test@example.com"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

#[tokio::test]
async fn run_parallel_runs_two_cards_at_once_in_two_worktrees_that_both_end_implemented() {
    let dir = tempfile::tempdir().unwrap();
    cargo_project(dir.path(), true);
    git_ok(dir.path(), &["init", "-q"]);
    git_ok(dir.path(), &["add", "-A"]);
    git_ok(dir.path(), &["commit", "-q", "-m", "probe"]);
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    for title in ["A", "B"] {
        run(&format!("/add task {title} parent 1"), &store, &worker()).await.unwrap();
    }
    // each run writes its own file: the workspace of each is the worktree, not the project. The
    // two runs overlap in an order the test does not fix, so the answer follows the request
    // rather than a script: a write first, `done` once the tool result is in the messages
    let script = std::sync::Arc::new(ByTurn { seen: parking_lot::Mutex::new(0) });
    let check: ra_tools::Script = std::sync::Arc::new(|_: &str| Ok((0, "ok\n".to_owned())));
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(script.clone()).with_script(check);

    let out = crate::agent::run_parallel(&store, &runner, 2, false).await.unwrap();
    let ran = implemented(&out);
    assert_eq!(ran.len(), 2, "{out:?}");
    assert_eq!(out.last().unwrap(), "ran 2 card(s) · no card is ready");
    for id in [2, 3] {
        // each run worked in its worktree on its branch (the check line names it), and the
        // walk landed each on the verdict: committed on the branch, merged, proved, accepted,
        // the worktree and the branch cleared
        assert_eq!(store.read(id).await.unwrap().state, ra_roadmap::model::TaskState::Accepted, "{out:?}");
        assert!(out.iter().any(|l| l.contains(&format!("branch card/{id}"))), "{out:?}");
        assert!(out.iter().any(|l| l == &format!("committed 1 file(s) on card/{id}")), "{out:?}");
        assert!(out.iter().any(|l| l.starts_with(&format!("merged card/{id} into "))), "{out:?}");
        let worktree = dir.path().join("var").join("worktrees").join(id.to_string());
        assert!(!worktree.join(".git").exists(), "{} was not cleared", worktree.display());
        assert!(git_ok(dir.path(), &["branch", "--list", &format!("card/{id}")]).trim().is_empty(), "card/{id} was not removed");
    }
    // the worker's file reached the project through the merges, not through the project's tree
    // (a checkout on Windows may end the line in CRLF)
    assert_eq!(std::fs::read_to_string(dir.path().join("made.txt")).unwrap().trim_end(), "by the worker");
    assert!(out.iter().any(|l| l == "check passed on the merged tree: cargo test --workspace -q"), "{out:?}");
    // the four completions went out as two runs of two turns, overlapping, then a verdict each
    assert_eq!(*script.seen.lock(), 6);
    assert_eq!(crate::agent::worktree_list(dir.path()).unwrap(), ["no worktree"]);
}

/// A provider that answers by the turn of the request: a `write_file` of `made.txt` on the
/// first turn of a run, `done` once a tool result is in the messages, `accepted` to a review
/// (which comes without tools); it counts the requests.
struct ByTurn {
    seen: parking_lot::Mutex<usize>,
}

impl ra_providers::ModelProvider for ByTurn {
    fn complete(&self, request: ra_providers::CompletionRequest) -> ra_providers::CompletionFuture<'_> {
        *self.seen.lock() += 1;
        let answered = request.messages.iter().any(|m| m.role == "tool");
        let answer = if request.tools.is_empty() {
            says("accepted", vec![])
        } else if answered {
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

/// Leaves the plan's five-hour window at `utilization` in the project's `var/limits.json`, seen
/// just now, so it is the latest reading on the machine.
fn plan_at(root: &std::path::Path, utilization: f64) {
    let now = crate::agent::unix_now();
    let mut limits = ra_providers::limits::Limits { status: "allowed".to_owned(), seen_at: now, ..Default::default() };
    limits.windows.insert("five_hour".to_owned(), ra_providers::limits::Window { utilization, resets_at: now + 3600 });
    limits.save(&root.join(ra_providers::claude_code::LIMITS_FILE)).unwrap();
}

/// Leaves the plan's week at `utilization` in the project's `var/limits.json`, seen just now,
/// resetting in six days and a half hour, so a slow suite still reads `6d 0h`.
fn week_at(root: &std::path::Path, utilization: f64) {
    let now = crate::agent::unix_now();
    let mut limits = ra_providers::limits::Limits { status: "allowed".to_owned(), seen_at: now, ..Default::default() };
    let resets_at = now + 6 * 86_400 + 1800;
    limits.windows.insert(ra_providers::limits::WEEK.to_owned(), ra_providers::limits::Window { utilization, resets_at });
    limits.save(&root.join(ra_providers::claude_code::LIMITS_FILE)).unwrap();
}

#[tokio::test]
async fn a_claude_run_past_the_weeks_threshold_is_refused_with_the_room_and_the_reset_unless_forced() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    week_at(dir.path(), 0.95);
    let script = Scripted::new(vec![says("did it", vec![])]);
    // a claude-* pick past 90%: refused before any token goes, naming the room and the reset
    let runner = Worker::new("claude-sonnet-5-5").with_provider(script.clone());
    let err = format!("{:#}", crate::agent::run(&store, 2, &runner).await.unwrap_err());
    assert!(err.contains("week is at 95%") && err.contains("past the 90% threshold"), "{err}");
    assert!(err.contains("5% left · resets in 6d 0h") && err.contains("--force"), "{err}");
    assert!(script.seen.lock().is_empty());
    assert_eq!(store.read(2).await.unwrap().state, ra_roadmap::model::TaskState::Defined);
    // another route is not the plan's
    assert!(Worker::new("z-ai/glm-5.3-flash").steered_by_plan(dir.path()).await.is_ok());
    // the project's own threshold stands above the week: room
    std::fs::write(dir.path().join("rhipper.toml"), "[plan]\nthreshold = 0.97\n").unwrap();
    assert_eq!(crate::agent::plan_threshold(dir.path()), 0.97);
    assert!(runner.steered_by_plan(dir.path()).await.is_ok());
    // forced, the run goes ahead on the badge model
    std::fs::remove_file(dir.path().join("rhipper.toml")).unwrap();
    assert!(runner.steered_by_plan(dir.path()).await.is_err());
    assert_eq!(runner.clone().with_force(true).steered_by_plan(dir.path()).await.unwrap().model, "claude-sonnet-5-5");
}

#[tokio::test]
async fn run_all_in_a_tight_window_takes_the_smallest_ready_card_first_and_the_start_line_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let store = three_cards(dir.path(), true).await;
    // #2 A is a c4 ahead of #3 B, a c1; #4 C, a c3, waits on #3
    store.set_complexity(2, 4).await.unwrap();
    store.set_complexity(3, 1).await.unwrap();
    let nodes = store.all().await.unwrap();
    assert_eq!(crate::agent::walk_order(&nodes, &[], false), vec![2, 3, 4]);
    assert_eq!(crate::agent::walk_order(&nodes, &[], true), vec![3, 4, 2]);
    // a tenth of the window left: the walk takes the c1 before the c4 that stands earlier
    plan_at(dir.path(), 0.9);
    let script = Scripted::new(accepted_runs(&["did B", "did C", "did A"]));
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(script.clone());
    let out = crate::agent::run_all(&store, &runner, false).await.unwrap();
    let ran = implemented(&out);
    assert_eq!(ran.len(), 3, "{out:?}");
    assert!(ran[0].starts_with("✓ #3 B implemented"), "{out:?}");
    assert!(ran[1].starts_with("✓ #4 C implemented"), "{out:?}");
    assert!(ran[2].starts_with("✓ #2 A implemented"), "{out:?}");
    let pair = crate::pair::recent(dir.path(), 20);
    assert!(pair.iter().any(|l| l.starts_with("start #3 B · glm-5.3-flash · tight: smallest first · check:")), "{pair:?}");
    // half the window left: the roadmap's order holds and the start line says nothing of it
    let dir = tempfile::tempdir().unwrap();
    let store = three_cards(dir.path(), true).await;
    store.set_complexity(2, 4).await.unwrap();
    store.set_complexity(3, 1).await.unwrap();
    plan_at(dir.path(), 0.5);
    let script = Scripted::new(accepted_runs(&["did A", "did B", "did C"]));
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(script.clone());
    let out = crate::agent::run_all(&store, &runner, false).await.unwrap();
    let ran = implemented(&out);
    assert!(ran[0].starts_with("✓ #2 A implemented"), "{out:?}");
    assert!(ran[1].starts_with("✓ #3 B implemented"), "{out:?}");
    let pair = crate::pair::recent(dir.path(), 20);
    assert!(pair.iter().any(|l| l.starts_with("start #2 A · glm-5.3-flash · check:")), "{pair:?}");
    assert!(!pair.iter().any(|l| l.contains("tight")), "{pair:?}");
}

#[tokio::test]
async fn run_all_stops_at_a_card_that_needs_revision_when_told_to() {
    let dir = tempfile::tempdir().unwrap();
    let store = three_cards(dir.path(), false).await;
    // the check always fails: the walk reruns #2 twice by itself, and the third failure waits
    let script = Scripted::new(vec![says("did A", vec![]), says("did A again", vec![]), says("did A once more", vec![])]);
    let failing: ra_tools::Script = std::sync::Arc::new(|_: &str| Ok((1, "the probe broke\n".to_owned())));
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(script.clone()).with_script(failing);
    let out = crate::agent::run_all(&store, &runner, true).await.unwrap();
    assert_eq!(out.last().unwrap(), "ran 3 card(s) · stopped: #2 needs revision");
    assert!(out.iter().any(|l| l.starts_with("rerun #2 A · its check failed 2 time(s)") && l.ends_with("(2 of 2)")), "{out:?}");
    assert!(out.iter().any(|l| l.starts_with("waits #2 A · its check failed 3 time(s) · waits for the person")), "{out:?}");
    assert_eq!(store.read(2).await.unwrap().state, ra_roadmap::model::TaskState::NeedsRevision);
    assert_eq!(store.read(3).await.unwrap().state, ra_roadmap::model::TaskState::Defined);
    assert_eq!(script.seen.lock().len(), 3);
}

/// Three cards in progress in a project whose activity log holds the starts given, none ended.
async fn orphaned_project(dir: &std::path::Path, activity: &str) -> Store {
    let store = Store::open(&dir.join("rhipper.md"), "Rhipper").await.unwrap();
    for title in ["A", "B", "C"] {
        run(&format!("/add task {title} parent 1"), &store, &worker()).await.unwrap();
    }
    for id in 2..=4 {
        store.set_state(id, ra_roadmap::model::TaskState::InProgress).await.unwrap();
    }
    std::fs::create_dir_all(dir.join("var")).unwrap();
    std::fs::write(dir.join(crate::ui::ACTIVITY_FILE), activity).unwrap();
    store
}

/// The feed lines beginning `re-queued`, without their stamps.
fn requeued_lines(dir: &std::path::Path) -> Vec<String> {
    crate::pair::recent(dir, 50).into_iter().filter(|l| l.starts_with("re-queued")).collect()
}

#[tokio::test]
async fn a_restart_requeues_the_orphaned_cards_once() {
    use crate::logs::RUN_START;
    use ra_roadmap::model::TaskState;
    let dir = tempfile::tempdir().unwrap();
    // #2 was started by two killed walks in a row: it is listed once
    let activity =
        format!("{RUN_START}#2 A · m · pid 11\n{RUN_START}#3 B · m · pid 12\n{RUN_START}#2 A · m · pid 13\n{RUN_START}#4 C · m · pid 14\n");
    let store = orphaned_project(dir.path(), &activity).await;
    store.set_session(2, Some(("s-2", "claude-code")), 1).await.unwrap();
    let line = crate::agent::requeue_orphans_with(&store, dir.path(), |_| false).await;
    assert_eq!(line.as_deref(), Some("re-queued 3 orphaned run(s): #2 #3 #4"));
    for id in 2..=4 {
        assert_eq!(store.read(id).await.unwrap().state, TaskState::Defined, "#{id}");
    }
    // the session stays, so the next run resumes it; the history says the run gave it back
    assert_eq!(store.read(2).await.unwrap().session.as_deref(), Some("s-2"));
    assert_eq!(store.history(Some(2)).unwrap().last().unwrap().by, ra_roadmap::history::Source::Run);
    // once: the next restart finds nothing and says nothing, and the feed shows no live run
    assert_eq!(crate::agent::requeue_orphans_with(&store, dir.path(), |_| false).await, None);
    assert_eq!(requeued_lines(dir.path()), ["re-queued 3 orphaned run(s): #2 #3 #4"]);
    assert_eq!(crate::pair::live_start(dir.path()), None);
}

#[tokio::test]
async fn an_orphan_that_ended_is_left_alone() {
    use crate::logs::{RUN_END, RUN_START};
    use ra_roadmap::model::TaskState;
    let dir = tempfile::tempdir().unwrap();
    // #2 ended, #3 is gone but its card moved on, #4 is still running: nothing goes back, so
    // no `re-queued` line lands and #4's start stays open on the feed
    let activity = format!("{RUN_START}#2 A · m · pid 11\n{RUN_END}✓ $0.01\n{RUN_START}#3 B · m · pid 12\n{RUN_START}#4 C · m · pid 14\n");
    let store = orphaned_project(dir.path(), &activity).await;
    store.set_state(3, TaskState::Review).await.unwrap();
    crate::pair::event(dir.path(), "start #4 C · m · pid 14");
    assert_eq!(crate::agent::requeue_orphans_with(&store, dir.path(), |pid| pid == 14).await, None);
    assert_eq!(store.read(2).await.unwrap().state, TaskState::InProgress);
    assert_eq!(store.read(3).await.unwrap().state, TaskState::Review);
    assert_eq!(store.read(4).await.unwrap().state, TaskState::InProgress);
    assert!(requeued_lines(dir.path()).is_empty());
    assert_eq!(crate::pair::live_start(dir.path()), Some(("#4 C · m · pid 14".to_owned(), Some(14))));
    // #3's orphan was closed in the activity log: the next restart does not see it again, and a
    // card left alone is named only beside the ones that went back
    assert_eq!(crate::pair::orphaned(dir.path(), |pid| pid == 14), Vec::<i64>::new());
}

#[tokio::test]
async fn a_stopped_walk_ends_after_the_current_card_with_its_rerun_still_queued() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    crate::agent::stop_walk(dir.path());
    let failing: ra_tools::Script = std::sync::Arc::new(|_: &str| Ok((1, "the probe broke\n".to_owned())));
    let provider = Scripted::new(vec![says("changed `src/lib.rs`", vec![])]);
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(provider.clone()).with_script(failing);
    let out = crate::agent::run_all(&store, &runner, false).await.unwrap();
    assert_eq!(out.last().unwrap(), "ran 1 card(s) · stopped after #2, as asked", "{out:?}");
    assert_eq!(provider.seen.lock().len(), 1);
}

#[tokio::test]
async fn a_walk_reruns_a_failed_check_and_a_first_rejection_by_itself_and_leaves_the_second_to_the_person() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    // the first check fails, every later one passes
    let checks = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = checks.clone();
    let script: ra_tools::Script = std::sync::Arc::new(move |command: &str| {
        if command.starts_with("cargo test") && counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            return Ok((1, "the probe broke\n".to_owned()));
        }
        Ok((0, "ok\n".to_owned()))
    });
    let provider = Scripted::new(vec![
        says("changed `src/lib.rs`", vec![]),
        says("changed `src/lib.rs` again", vec![]),
        says("needs_revision\n- src/lib.rs: the probe is not covered", vec![]),
        says("changed `src/lib.rs` to cover it", vec![]),
        says("needs_revision\n- src/lib.rs: still not covered", vec![]),
    ]);
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(provider.clone()).with_script(script);
    let out = crate::agent::run_all(&store, &runner, false).await.unwrap();
    // three runs of the one card with no `rc set` between them, then nothing is ready
    assert_eq!(out.last().unwrap(), "ran 3 card(s) · no card is ready", "{out:?}");
    assert!(out.iter().any(|l| l.starts_with("rerun #2 Probe · its check failed 1 time(s)") && l.ends_with("(1 of 2)")), "{out:?}");
    assert!(
        out.iter().any(|l| l.starts_with("rerun #2 Probe · the reviewer sent it back 1 time(s)") && l.ends_with("(1 of 1)")),
        "{out:?}"
    );
    let waits = "waits #2 Probe · the reviewer sent it back 2 time(s) · waits for the person: `rc set 2 defined` re-queues it";
    assert!(out.iter().any(|l| l == waits), "{out:?}");
    assert_eq!(store.read(2).await.unwrap().state, ra_roadmap::model::TaskState::NeedsRevision);
    // each rerun is a rerun: its prompt carries the last run's judgment
    let seen = provider.seen.lock();
    assert_eq!(seen.len(), 5);
    for rerun in [&seen[1], &seen[3]] {
        let prompt: String = rerun.messages.iter().map(|m| m.content.clone()).collect();
        assert!(prompt.contains("The last run of this card came back"), "{prompt}");
    }
    // the paired session reads that the card is the person's now
    let pair = crate::pair::recent(dir.path(), 30);
    assert!(pair.iter().any(|l| l == waits), "{pair:?}");
}

#[tokio::test]
async fn a_dry_run_prints_the_prompt_and_leaves_the_card_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    let card = store.read(2).await.unwrap();
    store.set_session(2, Some(("sess-1", &crate::agent::card_fingerprint(&card))), crate::agent::unix_now()).await.unwrap();
    let before = store.read(2).await.unwrap();
    // the harness route: the provider builds the command line and no process starts
    let (out, _) = run("/run 2 dry", &store, &Worker::new("claude-sonnet-5-5")).await.unwrap();
    let text = out.join("\n");
    assert!(out[0].contains("claude-sonnet-5-5"), "{text}");
    assert!(out.iter().any(|l| l.contains("effort medium") && l.contains("Bash(cargo *)")), "{text}");
    assert!(out.iter().any(|l| l == "  --resume") && out.iter().any(|l| l == "  sess-1"), "{text}");
    assert!(text.contains("--- system ---") && text.contains("--- user ---") && text.contains("Probe"), "{text}");
    // the check as the pre hook would run it: a cargo test without a tail gets one
    assert!(out.iter().any(|l| l.starts_with("  the pre hook runs `cargo test") && l.ends_with("2>&1 | tail -n 80`")), "{text}");
    let after = store.read(2).await.unwrap();
    assert_eq!(after.state, before.state);
    assert_eq!(after.session.as_deref(), Some("sess-1"));
    assert_eq!(after.notes, before.notes);
    // a bare model: the toolbox is what it would get
    let (out, _) = run("/run 2 dry", &store, &worker()).await.unwrap();
    assert!(out.iter().any(|l| l.contains("tools: read_file write_file")), "{}", out.join("\n"));
    assert_eq!(store.read(2).await.unwrap().state, before.state);
}

#[tokio::test]
async fn a_dry_run_shows_the_active_gambits_in_the_system_messages_and_not_the_parked_ones() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    store.add_gambit("Local first", 3, &["local".to_owned()]).await.unwrap();
    let parked = store.add_gambit("Polish the window", 2, &[]).await.unwrap();
    store.set_gambit_state(parked, ra_roadmap::model::GambitState::Parked).await.unwrap();
    let (out, _) = run("/run 2 dry", &store, &Worker::new("claude-sonnet-5-5")).await.unwrap();
    let system_end = out.iter().position(|l| l == "--- user ---").expect("the user turn follows the system messages");
    let system = out[..system_end].join("\n");
    assert!(system.contains(ra_context::DIRECTION_HEADING), "{system}");
    assert!(system.contains("- g1 Local first {w3} · focus: local"), "{system}");
    assert!(!system.contains("Polish the window"), "{system}");
    // a bare model reads it at the head of its task
    let (out, _) = run("/run 2 dry", &store, &worker()).await.unwrap();
    let text = out.join("\n");
    assert!(text.contains("- g1 Local first {w3} · focus: local") && !text.contains("Polish the window"), "{text}");
}

#[tokio::test]
async fn a_dry_run_sends_the_project_context_cut_to_the_card_s_crate_and_a_machine_note_once() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    std::fs::create_dir_all(dir.path().join("crates/ra-tools/src")).unwrap();
    std::fs::write(dir.path().join("crates/ra-tools/src/lib.rs"), "pub fn t() {}\n").unwrap();
    let project = "# Rhipper\n\nA project.\n\n## Layout\n\n- `crates/ra-roadmap` — the roadmap\n- `crates/ra-tools` — the tools\n- `crates/ra-usage` — the ledger\n\n## Build and checks\n\n- build with -p while the window runs\n";
    std::fs::write(dir.path().join(agent::PROJECT_FILE), project).unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    run("/files 2 crates/ra-tools/src/lib.rs", &store, &worker()).await.unwrap();
    // a look-back that says what the project context already says rides once; a new one rides
    crate::retro::record(
        dir.path(),
        Some(2),
        &crate::retro::parse("## Faster next time\n- [machine] build with -p while the window runs\n- [machine] sed is refused\n"),
    )
    .unwrap();
    let (out, _) = run("/run 2 dry", &store, &Worker::new("claude-sonnet-5-5")).await.unwrap();
    let system_end = out.iter().position(|l| l == "--- user ---").expect("the user turn follows the system messages");
    let system = out[..system_end].join("\n");
    assert!(system.contains("- `crates/ra-tools` — the tools") && system.contains("## Build and checks"), "{system}");
    assert!(!system.contains("ra-roadmap") && !system.contains("ra-usage"), "{system}");
    assert_eq!(system.matches("build with -p while the window runs").count(), 1, "{system}");
    assert!(system.contains("sed is refused"), "{system}");
    // a card with no files reads the whole context
    run("/add task Bare parent 1", &store, &worker()).await.unwrap();
    let (out, _) = run("/run 3 dry", &store, &Worker::new("claude-sonnet-5-5")).await.unwrap();
    assert!(out.join("\n").contains("- `crates/ra-usage` — the ledger"), "{}", out.join("\n"));
}

#[tokio::test]
async fn in_debug_mode_the_run_keeps_the_worker_s_look_back_for_the_next_one() {
    let dir = tempfile::tempdir().unwrap();
    cargo_project(dir.path(), true);
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    let report = "done\n\n## Faster next time\n- [machine] cargo test relinks rc.exe while the window runs: test with -p\n- the card named no files; a files line would have saved the search\n";
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(Scripted::new(vec![says(report, vec![]), says("accepted", vec![])]));
    assert!(runner.debug, "debug mode is on unless turned off");
    let (out, _) = run("/run 2", &store, &runner).await.unwrap();
    assert!(out.iter().any(|l| l.contains("↯ [machine] cargo test relinks")), "{out:?}");
    let kept = crate::retro::recent(dir.path(), 10);
    assert_eq!(kept.len(), 2);
    assert_eq!(kept[0].node, Some(2));
    // the next run reads them as machine notes
    assert!(crate::retro::machine_lines(dir.path()).contains("a files line"));
    // out of debug mode nothing is asked and nothing kept
    run("/add task Quiet parent 1", &store, &worker()).await.unwrap();
    let quiet = Worker::new("z-ai/glm-5.3-flash")
        .with_provider(Scripted::new(vec![says(report, vec![]), says("accepted", vec![])]))
        .with_debug(false);
    let (out, _) = run("/run 3", &store, &quiet).await.unwrap();
    assert!(!out.iter().any(|l| l.contains('↯')), "{out:?}");
    assert_eq!(crate::retro::recent(dir.path(), 10).len(), 2);
}

#[tokio::test]
async fn a_reviewer_with_points_leaves_the_card_needs_revision_with_the_points_first_in_its_notes() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    store.set_notes(2, "wrote hello.txt and read it back").await.unwrap();
    store.set_state(2, ra_roadmap::model::TaskState::Implemented).await.unwrap();
    let reviewer = Scripted::new(vec![says("needs_revision\n- hello.txt: the greeting lacks a newline\n- no test covers it", vec![])]);
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(reviewer.clone());

    let (out, _) = run("/review 2", &store, &runner).await.unwrap();
    assert!(out[0].starts_with("✗ #2 Probe needs revision (substantive) · 2 point(s) by glm-5.3-flash"), "{out:?}");
    assert_eq!(out[1], "  - hello.txt: the greeting lacks a newline");
    let node = store.read(2).await.unwrap();
    assert_eq!(node.state, ra_roadmap::model::TaskState::NeedsRevision);
    assert_eq!(
        node.notes.as_deref(),
        Some(
            "review by z-ai/glm-5.3-flash: needs revision (substantive)\n- hello.txt: the greeting lacks a newline\n- no test covers it\n\nwrote hello.txt and read it back"
        )
    );
    // the reviewer read the acceptance and the worker's report, with no tools and no workspace
    let seen = reviewer.seen.lock();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].tools.is_empty() && seen[0].workspace.is_none());
    assert!(seen[0].messages[1].content.contains("Acceptance:"), "{}", seen[0].messages[1].content);
    assert!(seen[0].messages[1].content.contains("wrote hello.txt and read it back"), "{}", seen[0].messages[1].content);
}

#[tokio::test]
async fn a_reviewer_that_accepts_moves_the_card_to_accepted_and_the_verdict_leads_the_notes() {
    let store = store().await;
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    let reviewer = Scripted::new(vec![says("Accepted.", vec![])]);
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(reviewer);
    let (out, _) = run("/review 2", &store, &runner).await.unwrap();
    assert!(out[0].starts_with("✓ #2 Probe accepted by glm-5.3-flash"), "{out:?}");
    let node = store.read(2).await.unwrap();
    assert_eq!(node.state, ra_roadmap::model::TaskState::Accepted);
    assert_eq!(node.notes.as_deref(), Some("review by z-ai/glm-5.3-flash: accepted"));
    use crate::agent::{Verdict, verdict_of};
    assert_eq!(verdict_of("needs revision\n* a\n* b"), Verdict::NeedsRevision(vec!["a".into(), "b".into()]));
    assert_eq!(verdict_of("the diff misses the test"), Verdict::NeedsRevision(vec!["the diff misses the test".into()]));
}

/// Reviews a card that has one handoff of a run with `answer` as the reviewer's verdict: the
/// run line, the notes and the outcome the handoff was left with.
async fn reviewed_after_a_run(answer: &str) -> (Vec<String>, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    store.set_notes(2, "wrote hello.txt").await.unwrap();
    store.set_state(2, ra_roadmap::model::TaskState::Implemented).await.unwrap();
    let mut task = ra_context::TaskContext::open(dir.path(), 2);
    task.record(ra_context::Handoff {
        when: 7,
        model: "z-ai/glm-5.3-flash".to_owned(),
        outcome: "implemented".to_owned(),
        check: "cargo test -q: passed".to_owned(),
        verdict: None,
        changed: vec!["hello.txt".to_owned()],
        report: "wrote hello.txt".to_owned(),
        warnings: Vec::new(),
        read: Vec::new(),
    });
    task.save(dir.path()).unwrap();
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(Scripted::new(vec![says(answer, vec![])]));
    let (out, _) = run("/review 2", &store, &runner).await.unwrap();
    assert_eq!(store.read(2).await.unwrap().state, ra_roadmap::model::TaskState::NeedsRevision);
    let notes = store.read(2).await.unwrap().notes.unwrap();
    let handoff = ra_context::TaskContext::open(dir.path(), 2).handoffs.pop().unwrap();
    // the rest of the handoff is as the run left it
    assert_eq!(handoff.changed, vec!["hello.txt".to_owned()]);
    assert_eq!(handoff.check, "cargo test -q: passed");
    (out, notes, handoff.outcome)
}

#[tokio::test]
async fn a_lint_only_verdict_is_trivial_and_the_tag_lands_on_the_handoff() {
    let (out, notes, outcome) = reviewed_after_a_run(
        "needs_revision\n- src/lib.rs: clippy warns of a needless clone at line 12\n- src/lib.rs: rustfmt would rewrap the match",
    )
    .await;
    assert!(out[0].starts_with("✗ #2 Probe needs revision (trivial) · 2 point(s)"), "{out:?}");
    assert!(notes.starts_with("review by z-ai/glm-5.3-flash: needs revision (trivial)\n- src/lib.rs: clippy"), "{notes}");
    assert_eq!(outcome, "needs revision (trivial)");
}

#[tokio::test]
async fn a_doc_comment_verdict_is_trivial() {
    let (out, _, outcome) = reviewed_after_a_run("needs_revision\n- src/lib.rs: `parse` has no doc comment").await;
    assert!(out[0].contains("needs revision (trivial)"), "{out:?}");
    assert_eq!(outcome, "needs revision (trivial)");
}

#[tokio::test]
async fn a_logic_verdict_stays_substantive_even_beside_a_lint_point() {
    let (out, _, outcome) = reviewed_after_a_run(
        "needs_revision\n- src/lib.rs: clippy warns of an unused variable\n- src/lib.rs: `parse` returns the wrong id for a nested card",
    )
    .await;
    assert!(out[0].contains("needs revision (substantive)"), "{out:?}");
    assert_eq!(outcome, "needs revision (substantive)");
    use crate::commands::review::{Weight, classify};
    let points = |list: &[&str]| list.iter().map(|p| (*p).to_owned()).collect::<Vec<_>>();
    assert_eq!(classify(&points(&["no test covers the empty case"])), Weight::Substantive);
    assert_eq!(classify(&points(&["the greeting lacks a newline"])), Weight::Substantive);
    assert_eq!(classify(&points(&["the reviewer gave no points"])), Weight::Substantive);
    assert_eq!(classify(&points(&["missing docs on the `Weight` enum", "cargo fmt differs in agent.rs"])), Weight::Trivial);
}

#[tokio::test]
async fn a_dirty_file_from_before_the_run_is_not_the_cards() {
    let dir = tempfile::tempdir().unwrap();
    git_ok(dir.path(), &["init", "-q"]);
    std::fs::write(dir.path().join("other.txt"), "old\n").unwrap();
    std::fs::write(dir.path().join("mine.txt"), "old\n").unwrap();
    git_ok(dir.path(), &["add", "-A"]);
    git_ok(dir.path(), &["commit", "-q", "-m", "probe"]);
    // the person left both files dirty before the run, their times older than it
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    for (name, text) in [("other.txt", "edited by the person\n"), ("mine.txt", "the person's start\n")] {
        let path = dir.path().join(name);
        std::fs::write(&path, text).unwrap();
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(old).unwrap();
    }
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    // the card names a file the run leaves alone: the reviewer still sees it listed
    run("/files 2 src/lib.rs", &store, &worker()).await.unwrap();
    // the run writes one of the dirty files and its report names both: only the one it wrote
    // inside the run is the card's (#157 kept a named dirty file, #185 was judged on the other)
    let write = ra_providers::ToolCall {
        id: "call_1".into(),
        name: "write_file".into(),
        arguments: r#"{"path":"mine.txt","content":"written by the worker\n"}"#.into(),
    };
    let script = Scripted::new(vec![
        says("", vec![write]),
        says("edited `mine.txt`, looked at `other.txt` and left `missing.txt` alone", vec![]),
        says("accepted", vec![]),
    ]);
    let check: ra_tools::Script = std::sync::Arc::new(|_: &str| Ok((0, "ok\n".to_owned())));
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(script.clone()).with_script(check);
    let (out, _) = run("/run 2", &store, &runner).await.unwrap();
    assert!(out[0].starts_with("✓ #2 Probe implemented"), "{out:?}");
    let handoff = ra_context::TaskContext::open(dir.path(), 2).handoffs.pop().unwrap();
    assert_eq!(handoff.changed, vec!["mine.txt".to_owned()]);
    let seen = script.seen.lock();
    assert_eq!(seen.len(), 3);
    let brief = &seen[2].messages.last().unwrap().content;
    assert!(brief.contains("+written by the worker"), "{brief}");
    // the other dirty file is listed apart and kept out of the diff
    let apart = brief.find(&format!("## {}", crate::review::NOT_THE_CARDS)).expect("the not-this-card heading");
    assert!(brief[apart..].contains("\n- other.txt\n"), "{brief}");
    assert!(!brief.contains("+edited by the person"), "{brief}");
    // the brief lists the card's files and the run's, then the stat of exactly those, before
    // the diff: a change in a file the reviewer did not expect is not a missing one (#159)
    assert!(brief.contains("## Files of the card\n- src/lib.rs\n- mine.txt (changed by the run, not on the card)\n"), "{brief}");
    let stat = brief.find("## git diff --stat HEAD of the card's files\n").expect("the stat section");
    assert!(brief[stat..].contains("mine.txt | 2 +-"), "{brief}");
    assert!(!brief[stat..].contains("other.txt |"), "{brief}");
    assert!(stat < brief.find("## git diff HEAD of the card's files\n").unwrap(), "{brief}");
    assert!(seen[2].messages[0].content.contains("with their diff stat"), "{}", seen[2].messages[0].content);
    // the worker's instruction asks for the file of each change, which is what the list reads
    assert!(seen[0].messages[0].content.contains("naming the file each change landed in"), "{}", seen[0].messages[0].content);
}

#[tokio::test]
async fn a_worktree_review_diffs_the_branch_from_its_merge_base() {
    let dir = tempfile::tempdir().unwrap();
    git_ok(dir.path(), &["init", "-q"]);
    std::fs::write(dir.path().join(".gitignore"), "var/\nrhipper.md\n").unwrap();
    git_ok(dir.path(), &["add", "-A"]);
    git_ok(dir.path(), &["commit", "-q", "-m", "base"]);
    let tree = dir.path().join("var/worktrees/2");
    git_ok(dir.path(), &["worktree", "add", "-q", "-b", "card/2", tree.to_str().unwrap()]);
    // two runs of the card, each checkpointed on its branch
    for name in ["first.txt", "second.txt"] {
        std::fs::write(tree.join(name), "work\n").unwrap();
        git_ok(&tree, &["add", "-A"]);
        git_ok(&tree, &["commit", "-q", "-m", name]);
    }
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    // the last run's handoff names only its own file
    let mut task = ra_context::TaskContext::open(dir.path(), 2);
    task.record(ra_context::Handoff {
        when: 7,
        model: "z-ai/glm-5.3-flash".to_owned(),
        outcome: "implemented".to_owned(),
        check: "cargo test -q: passed".to_owned(),
        verdict: None,
        changed: vec!["second.txt".to_owned()],
        report: "wrote `second.txt`".to_owned(),
        warnings: Vec::new(),
        read: Vec::new(),
    });
    task.save(dir.path()).unwrap();
    let node = store.read(2).await.unwrap();
    let brief = crate::agent::review_brief(dir.path(), &tree, &node).await;
    assert!(brief.contains("- first.txt (changed by the run, not on the card)\n- second.txt"), "{brief}");
    let stat = brief.find("## git diff --stat ").expect("the stat section");
    assert!(!brief[stat..].starts_with("## git diff --stat HEAD "), "the base is the merge base: {brief}");
    assert!(brief[stat..].contains("first.txt") && brief[stat..].contains("second.txt"), "{brief}");
    assert!(!brief.contains(crate::review::NOT_THE_CARDS), "{brief}");
}

/// A trivial verdict of the reviewer.
const TRIVIAL_VERDICT: &str = "needs_revision\n- src/lib.rs: `probe` has no doc comment";

/// Reviews an implemented card with one handoff in a project whose `[role]` table puts the fix
/// pass on the scripted model and whose checks are scripted to pass; `answers` is what the
/// model says in turn: the verdict, the fix pass, the second verdict. The lines, the store, the
/// script and the project directory come back.
async fn reviewed_with_a_fix_role(answers: Vec<&str>) -> (Vec<String>, Store, std::sync::Arc<Scripted>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(crate::router::ROUTES_FILE), "[role]\nfix = \"z-ai/glm-5.3-flash\"\n").unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    store.set_notes(2, "wrote hello.txt").await.unwrap();
    store.set_state(2, ra_roadmap::model::TaskState::Implemented).await.unwrap();
    let mut task = ra_context::TaskContext::open(dir.path(), 2);
    task.record(ra_context::Handoff {
        when: 7,
        model: "claude-fable-5-1".to_owned(),
        outcome: "implemented".to_owned(),
        check: "cargo test -q: passed".to_owned(),
        verdict: None,
        changed: vec!["hello.txt".to_owned()],
        report: "wrote hello.txt".to_owned(),
        warnings: Vec::new(),
        read: Vec::new(),
    });
    task.save(dir.path()).unwrap();
    let script: ra_tools::Script = std::sync::Arc::new(|_: &str| Ok((0, "ok\n".to_owned())));
    let model = Scripted::new(answers.into_iter().map(|answer| says(answer, vec![])).collect());
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(model.clone()).with_script(script).with_debug(false);
    let (out, _) = run("/review 2", &store, &runner).await.unwrap();
    (out, store, model, dir)
}

#[tokio::test]
async fn a_trivial_rejection_gets_one_cheap_fix_pass_from_the_verdict_and_is_judged_again() {
    let (out, store, model, dir) = reviewed_with_a_fix_role(vec![TRIVIAL_VERDICT, "added the doc comment on `probe`", "accepted"]).await;
    // reject → fix → re-judge → accepted, in that order on the lines
    assert!(out[0].starts_with("✗ #2 Probe needs revision (trivial) · 1 point(s)"), "{out:?}");
    let fix = out.iter().position(|l| l == "  fix pass on glm-5.3-flash").expect("the fix pass line");
    let fixed = out.iter().position(|l| l.starts_with("✓ #2 Probe implemented on glm-5.3-flash")).expect("the fix pass's run line");
    let accepted = out.iter().position(|l| l.starts_with("✓ #2 Probe accepted by glm-5.3-flash")).expect("the second verdict");
    assert!(fix < fixed && fixed < accepted, "{out:?}");
    assert!(!out.iter().any(|l| l.contains("waits for the person")), "{out:?}");
    let node = store.read(2).await.unwrap();
    assert_eq!(node.state, ra_roadmap::model::TaskState::Accepted);
    assert!(node.notes.as_deref().unwrap().starts_with("review by z-ai/glm-5.3-flash: accepted"), "{:?}", node.notes);
    // the verdict led the fix pass's task; the checks ran again before the second verdict
    let seen = model.seen.lock();
    assert_eq!(seen.len(), 3);
    let prompt = &seen[1].messages.last().unwrap().content;
    assert!(prompt.contains("A reviewer sent this card back"), "{prompt}");
    assert!(prompt.contains("- src/lib.rs: `probe` has no doc comment"), "{prompt}");
    assert!(seen[1].workspace.is_some() && !seen[1].tools.is_empty(), "the fix pass acts in the workspace with tools");
    assert!(seen[2].tools.is_empty(), "the second verdict is a review");
    // the handoffs: the run, tagged trivial, then the fix pass on the cheap model
    let handoffs = ra_context::TaskContext::open(dir.path(), 2).handoffs;
    assert_eq!(handoffs.len(), 2);
    assert_eq!(handoffs[0].outcome, "needs revision (trivial)");
    assert_eq!(handoffs[1].model, "z-ai/glm-5.3-flash");
    assert_eq!(handoffs[1].outcome, "implemented");
}

#[tokio::test]
async fn a_second_trivial_rejection_halts_and_the_card_waits_for_the_person() {
    let (out, store, model, dir) =
        reviewed_with_a_fix_role(vec![TRIVIAL_VERDICT, "added the doc comment on `probe`", TRIVIAL_VERDICT]).await;
    assert_eq!(out.last().unwrap(), "  a second trivial rejection: the card waits for the person");
    assert_eq!(out.iter().filter(|l| l.starts_with("✗ #2 Probe needs revision (trivial)")).count(), 2, "{out:?}");
    assert_eq!(store.read(2).await.unwrap().state, ra_roadmap::model::TaskState::NeedsRevision);
    // no third pass: the model was asked three times, the verdict, the fix and the verdict
    assert_eq!(model.seen.lock().len(), 3);
    let handoffs = ra_context::TaskContext::open(dir.path(), 2).handoffs;
    assert_eq!(handoffs.iter().filter(|h| h.outcome == "needs revision (trivial)").count(), 2);
    // a later review of the same card runs no fix pass either
    let again = Scripted::new(vec![says(TRIVIAL_VERDICT, vec![])]);
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(again.clone());
    let (out, _) = run("/review 2", &store, &runner).await.unwrap();
    assert_eq!(out.last().unwrap(), "  a second trivial rejection: no fix pass, the card waits for the person");
    assert_eq!(again.seen.lock().len(), 1);
}

#[tokio::test]
async fn a_trivial_rejection_without_a_fix_model_waits_for_the_person() {
    // no `[role] fix` and no server in the tests: the verdict stands and nothing else is called
    let (out, _, outcome) = reviewed_after_a_run(TRIVIAL_VERDICT).await;
    assert_eq!(out.last().unwrap(), "  the fix role is off: the card waits for the person");
    assert_eq!(outcome, "needs revision (trivial)");
}

/// A run of card #2 in a project whose check passes, with `answers` as what the model says in
/// turn: the worker's report, then the reviewer's verdict. The lines, the node after the run and
/// the handoff the run left.
async fn judged_run(answers: Vec<&str>) -> (Vec<String>, ra_roadmap::model::Node, ra_context::Handoff, std::sync::Arc<Scripted>) {
    let dir = tempfile::tempdir().unwrap();
    cargo_project(dir.path(), true);
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    let model = Scripted::new(answers.into_iter().map(|answer| says(answer, vec![])).collect());
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(model.clone());
    let (out, _) = run("/run 2", &store, &runner).await.unwrap();
    let handoff = ra_context::TaskContext::open(dir.path(), 2).handoffs.pop().expect("the run's handoff");
    (out, store.read(2).await.unwrap(), handoff, model)
}

#[tokio::test]
async fn a_run_that_passes_its_checks_is_judged_and_an_accepting_verdict_leaves_it_implemented_with_the_verdict_on_the_handoff() {
    let (out, node, handoff, model) = judged_run(vec!["done", "accepted"]).await;
    // the run line, then the verdict's
    assert!(out[0].starts_with("✓ #2 Probe implemented on glm-5.3-flash"), "{out:?}");
    assert!(out.iter().any(|l| l.starts_with("✓ #2 Probe accepted by glm-5.3-flash")), "{out:?}");
    // the card is offered to the person, not accepted behind their back; the notes stay the
    // worker's and the handoff carries the verdict
    assert_eq!(node.state, ra_roadmap::model::TaskState::Implemented);
    assert_eq!(node.notes.as_deref(), Some("done"));
    assert_eq!(handoff.outcome, "implemented");
    assert_eq!(handoff.verdict.as_deref(), Some("accepted by z-ai/glm-5.3-flash"));
    // the reviewer was the second call, without tools, and read the card and the handoff
    let seen = model.seen.lock();
    assert_eq!(seen.len(), 2);
    assert!(seen[1].tools.is_empty() && seen[1].workspace.is_none());
    assert!(
        seen[1].messages[1].content.contains("## Handoff of the last run\nimplemented on z-ai/glm-5.3-flash"),
        "{}",
        seen[1].messages[1].content
    );
}

#[tokio::test]
async fn a_run_whose_reviewer_rejects_it_ends_needs_revision_with_the_points_and_the_verdict_on_the_handoff() {
    let (out, node, handoff, _) = judged_run(vec!["done", "needs_revision\n- src/lib.rs: no test covers the empty case"]).await;
    assert!(out.iter().any(|l| l.starts_with("✗ #2 Probe needs revision (substantive) · 1 point(s) by glm-5.3-flash")), "{out:?}");
    assert_eq!(node.state, ra_roadmap::model::TaskState::NeedsRevision);
    assert_eq!(
        node.notes.as_deref(),
        Some("review by z-ai/glm-5.3-flash: needs revision (substantive)\n- src/lib.rs: no test covers the empty case\n\ndone")
    );
    assert_eq!(handoff.outcome, "needs revision (substantive)");
    assert_eq!(handoff.verdict.as_deref(), Some("needs revision (substantive) · 1 point(s) by z-ai/glm-5.3-flash"));
}

#[tokio::test]
async fn a_run_whose_reviewer_does_not_answer_waits_in_review_without_a_verdict_and_is_not_offered() {
    // the script ends with the worker's report: the reviewer's call fails
    let (out, node, handoff, _) = judged_run(vec!["done"]).await;
    assert!(out.iter().any(|l| l.starts_with("  no verdict on #2: ") && l.ends_with("`review 2` judges it")), "{out:?}");
    assert_eq!(node.state, ra_roadmap::model::TaskState::Review);
    assert_eq!(node.notes.as_deref(), Some("done"));
    assert_eq!(handoff.outcome, "implemented");
    assert_eq!(handoff.verdict, None);
}

#[tokio::test]
async fn a_planners_cards_are_added_under_the_milestone_only_on_confirmation() {
    let store = store().await;
    run("/add milestone M parent 1", &store, &worker()).await.unwrap();
    store.set_details(2, Some("ship the greeting"), None).await.unwrap();
    run("/add task Old parent 2", &store, &worker()).await.unwrap();
    let planner = Scripted::new(vec![says(
        "```\n- Write the greeting {c2}\n  objective: hello.txt holds the greeting\n  acceptance: `cargo test -p greet` passes\n  files: src/greet.rs hello.txt\n- Test it {c1}\n  objective: a test reads it back\n  acceptance: `cargo test -p greet` passes\n- Document it\n  the README names the file\n```",
        vec![],
    )]);
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(planner.clone());

    let plan = crate::agent::plan(&store, 2, &runner).await.unwrap();
    assert!(plan.preview()[0].starts_with("3 tasks proposed under #2 M by glm-5.3-flash"), "{:?}", plan.preview());
    // nothing is added until the person says so
    assert_eq!(store.all().await.unwrap().len(), 3);
    // the planner read the context of the milestone and the card already under it, with no tools
    let seen = planner.seen.lock();
    assert!(seen[0].tools.is_empty() && seen[0].workspace.is_none());
    assert!(seen[0].messages[1].content.contains("ship the greeting"), "{}", seen[0].messages[1].content);
    assert!(seen[0].messages[1].content.contains("#3 task Old"), "{}", seen[0].messages[1].content);
    drop(seen);

    let out = plan.add(&store).await.unwrap();
    assert_eq!(out.last().unwrap(), "added 3 tasks under #2 M");
    let nodes = store.all().await.unwrap();
    assert_eq!(nodes.len(), 6);
    let first = nodes.iter().find(|n| n.id == 4).unwrap();
    assert_eq!((first.parent, first.kind, first.complexity), (Some(2), ra_roadmap::model::NodeKind::Task, 2));
    assert_eq!(first.objective.as_deref(), Some("hello.txt holds the greeting"));
    assert_eq!(first.acceptance.as_deref(), Some("`cargo test -p greet` passes"));
    assert_eq!(first.files, vec!["src/greet.rs".to_owned(), "hello.txt".to_owned()]);
    let third = nodes.iter().find(|n| n.id == 6).unwrap();
    assert_eq!((third.title.as_str(), third.complexity, third.objective.as_deref()), ("Document it", 3, Some("the README names the file")));
}

/// The report of the role tests: an answer, and a look back whose bullet lost its tag.
const ROLE_REPORT: &str = "wrote the greeting into hello.txt and verified it by cargo test\n\n## Faster next time\n- Select-String after a native pipe is refused here; grep in Bash works\n";

#[tokio::test]
async fn the_summary_role_runs_on_the_local_provider_and_not_on_the_cards_model() {
    let dir = tempfile::tempdir().unwrap();
    cargo_project(dir.path(), true);
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    // the card's model: the report, then the verdict on it
    let card = Scripted::new(vec![says(ROLE_REPORT, vec![]), says("accepted", vec![])]);
    // the roles in the order a run asks them: the summary, the classification, the digest
    let local = Scripted::new(vec![
        says("wrote hello.txt; verified by cargo test", vec![]),
        says("machine", vec![]),
        says("#2 Probe implemented: hello.txt written\nnothing else", vec![]),
    ]);
    let ledger = ra_usage::UsageStore::in_memory().await.unwrap();
    let runner = Worker::new("z-ai/glm-5.3-flash")
        .with_provider(card.clone())
        .with_role_provider("local/qwen3-8b", local.clone())
        .with_ledger(ledger);
    let (out, _) = run("/run 2", &store, &runner).await.unwrap();
    // the card's model answered as the worker and as the reviewer; every cheap role went to the
    // local provider
    assert_eq!(card.seen.lock().len(), 2);
    {
        let seen = local.seen.lock();
        assert_eq!(seen.len(), 3, "{out:?}");
        assert!(seen.iter().all(|r| r.model == "local/qwen3-8b" && r.tools.is_empty() && r.workspace.is_none()));
        assert!(seen[0].messages[1].content.starts_with("wrote the greeting"), "{}", seen[0].messages[1].content);
        assert_eq!(seen[1].messages[1].content, "Select-String after a native pipe is refused here; grep in Bash works");
        assert!(seen[2].messages[1].content.starts_with("✓ #2 Probe implemented"), "{}", seen[2].messages[1].content);
    }
    // the handoff carries the summary, the notes of the card the whole answer
    let task = ra_context::TaskContext::open(dir.path(), 2);
    assert_eq!(task.handoffs.last().unwrap().report, "wrote hello.txt; verified by cargo test");
    assert_eq!(store.read(2).await.unwrap().notes.as_deref(), Some(ROLE_REPORT));
    // the untagged bullet was classified as the machine's
    assert!(out.iter().any(|l| l.contains("↯ [machine] Select-String")), "{out:?}");
    assert!(crate::retro::recent(dir.path(), 1)[0].machine);
    // the digest line is the drafted one, its first line only
    assert_eq!(crate::hub::own_digest(dir.path()).unwrap().last.as_deref(), Some("#2 Probe implemented: hello.txt written"));
    // the ledger shows the roles' calls under the local model, apart from the card's
    let rows = runner.by_model().await.unwrap();
    assert_eq!(rows.len(), 2, "{rows:?}");
    let local_row = rows.iter().find(|(model, _)| model == "local/qwen3-8b").unwrap();
    assert_eq!(local_row.1.calls, 3);
    assert_eq!(rows.iter().find(|(model, _)| model == "z-ai/glm-5.3-flash").unwrap().1.calls, 2);
    assert_eq!(runner.tally().calls, 5);
    // each row of the card names its role: the worker's none, the reviewer's and the three
    // cheap roles theirs, so `rc usage` lists them apart
    let calls = runner.calls_of(Some(2)).await.unwrap();
    let mut tags: Vec<(&str, Option<&str>)> = calls.iter().map(|c| (c.model.as_str(), c.role.as_deref())).collect();
    tags.sort_unstable();
    assert_eq!(
        tags,
        [
            ("local/qwen3-8b", Some("classify")),
            ("local/qwen3-8b", Some("digest")),
            ("local/qwen3-8b", Some("summary")),
            ("z-ai/glm-5.3-flash", None),
            ("z-ai/glm-5.3-flash", Some("reviewer")),
        ]
    );
    let by_role = runner.by_role().await.unwrap();
    assert_eq!(by_role.len(), 5, "{by_role:?}");
    assert!(by_role.iter().all(|(_, _, t)| t.calls == 1));
}

#[test]
fn the_cheap_roles_take_the_tables_model_else_the_local_server_and_make_no_call_without_one() {
    use ra_providers::local::Role;
    let config: crate::router::RoleConfig = toml::from_str("summary = \"z-ai/glm-5.3-flash\"\nclassify = \"\"\n").unwrap();
    // the server answered: the named role keeps its model, the one left out goes local, the
    // one turned off stays off
    assert_eq!(
        crate::roles::Roles::resolve(&config, Some("local/qwen3-8b")),
        [(Role::Summary, "z-ai/glm-5.3-flash".to_owned()), (Role::Digest, "local/qwen3-8b".to_owned())]
    );
    // no server: the role left out makes no call, and never falls to a frontier model
    assert_eq!(crate::roles::Roles::resolve(&config, None), [(Role::Summary, "z-ai/glm-5.3-flash".to_owned())]);
    assert!(crate::roles::Roles::resolve(&crate::router::RoleConfig::default(), None).is_empty());
    assert_eq!(crate::roles::Roles::resolve(&crate::router::RoleConfig::default(), Some("local/qwen3-8b")).len(), 3);
}

#[tokio::test]
async fn without_a_local_server_the_roles_make_no_call_and_the_rule_based_path_stands() {
    let dir = tempfile::tempdir().unwrap();
    cargo_project(dir.path(), true);
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    let card = Scripted::new(vec![says(ROLE_REPORT, vec![]), says("accepted", vec![])]);
    let ledger = ra_usage::UsageStore::in_memory().await.unwrap();
    // no role provider and no server: a run resolves no role at all; the card's model is the
    // worker and the reviewer, nothing else is called
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(card.clone()).with_ledger(ledger);
    let (out, _) = run("/run 2", &store, &runner).await.unwrap();
    assert_eq!(card.seen.lock().len(), 2);
    assert_eq!(runner.tally().calls, 2);
    assert_eq!(runner.by_model().await.unwrap().len(), 1);
    // the handoff carries the answer itself, the bullet stays as the worker tagged it, and
    // the digest line is the feed's end line
    let task = ra_context::TaskContext::open(dir.path(), 2);
    let report = &task.handoffs.last().unwrap().report;
    assert!(report.starts_with("wrote the greeting into hello.txt") && report.ends_with("grep in Bash works"), "{report}");
    assert!(out.iter().any(|l| l.contains("↯ Select-String")), "{out:?}");
    assert!(!crate::retro::recent(dir.path(), 1)[0].machine);
    assert!(crate::hub::own_digest(dir.path()).unwrap().last.unwrap().starts_with("✓ #2 Probe implemented"));
}

#[tokio::test]
async fn a_role_without_a_local_server_makes_no_call() {
    let dir = tempfile::tempdir().unwrap();
    cargo_project(dir.path(), true);
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    let card = Scripted::new(vec![says(ROLE_REPORT, vec![]), says("accepted", vec![])]);
    let ledger = ra_usage::UsageStore::in_memory().await.unwrap();
    // no `[role]` table and no server: every role resolves to off, and the start line says so
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(card.clone()).with_ledger(ledger);
    assert_eq!(runner.roles(dir.path()).await.summary(), "roles: summary off · digest off · classify off");
    run("/run 2", &store, &runner).await.unwrap();
    // the card's model answered as the worker and the reviewer; no role fell to a Claude model,
    // so the ledger has no claude row and no row tagged with a cheap role
    assert_eq!(card.seen.lock().len(), 2);
    assert_eq!(runner.tally().calls, 2);
    let rows = runner.by_model().await.unwrap();
    assert!(rows.iter().all(|(model, _)| !model.starts_with("claude-")), "{rows:?}");
    let calls = runner.calls_of(Some(2)).await.unwrap();
    let tags: Vec<(&str, Option<&str>)> = calls.iter().map(|c| (c.model.as_str(), c.role.as_deref())).collect();
    assert!(tags.iter().all(|(_, role)| !matches!(role, Some("summary" | "digest" | "classify"))), "{tags:?}");
}

#[tokio::test]
async fn a_role_named_in_the_table_goes_to_that_model() {
    let dir = tempfile::tempdir().unwrap();
    cargo_project(dir.path(), true);
    // the table names the summary's model and turns the other two off; nothing pings a server
    std::fs::write(
        dir.path().join(crate::router::ROUTES_FILE),
        "[role]\nsummary = \"z-ai/glm-5.3-flash\"\ndigest = \"\"\nclassify = \"\"\n",
    )
    .unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    // the named model is the card's own, so its provider takes the report, the summary, then
    // the verdict
    let card =
        Scripted::new(vec![says(ROLE_REPORT, vec![]), says("wrote hello.txt; verified by cargo test", vec![]), says("accepted", vec![])]);
    let ledger = ra_usage::UsageStore::in_memory().await.unwrap();
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(card.clone()).with_ledger(ledger);
    assert_eq!(runner.roles(dir.path()).await.summary(), "roles: summary z-ai/glm-5.3-flash · digest off · classify off");
    let (out, _) = run("/run 2", &store, &runner).await.unwrap();
    {
        let seen = card.seen.lock();
        assert_eq!(seen.len(), 3, "{out:?}");
        assert!(seen[1].tools.is_empty() && seen[1].workspace.is_none());
        assert!(seen[1].messages[1].content.starts_with("wrote the greeting"), "{}", seen[1].messages[1].content);
    }
    assert_eq!(ra_context::TaskContext::open(dir.path(), 2).handoffs.last().unwrap().report, "wrote hello.txt; verified by cargo test");
    // the role's row carries the role in its model column, as the reviewer's does
    let calls = runner.calls_of(Some(2)).await.unwrap();
    let mut tags: Vec<(&str, Option<&str>)> = calls.iter().map(|c| (c.model.as_str(), c.role.as_deref())).collect();
    tags.sort_unstable();
    assert_eq!(tags, [("z-ai/glm-5.3-flash", None), ("z-ai/glm-5.3-flash", Some("reviewer")), ("z-ai/glm-5.3-flash", Some("summary"))]);
    assert!(runner.by_model().await.unwrap().iter().all(|(model, _)| !model.starts_with("claude-")));
}

#[tokio::test]
async fn a_local_server_that_went_down_degrades_the_roles_to_no_call_and_never_to_the_cards_model() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let dir = tempfile::tempdir().unwrap();
    cargo_project(dir.path(), true);
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    let card = Scripted::new(vec![says(ROLE_REPORT, vec![]), says("accepted", vec![])]);
    // the stub server is down: it takes each connection and hangs up without an answer, and
    // counts them, so the test sees how many times the roles reached for it
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://127.0.0.1:{}/v1", listener.local_addr().unwrap().port());
    let reached = std::sync::Arc::new(AtomicUsize::new(0));
    let counter = reached.clone();
    std::thread::spawn(move || {
        for socket in listener.incoming().flatten() {
            counter.fetch_add(1, Ordering::SeqCst);
            drop(socket);
        }
    });
    let local: std::sync::Arc<dyn ra_providers::ModelProvider> =
        std::sync::Arc::new(ra_providers::openrouter::OpenRouter::at(&url, "").unwrap());
    let ledger = ra_usage::UsageStore::in_memory().await.unwrap();
    let runner =
        Worker::new("z-ai/glm-5.3-flash").with_provider(card.clone()).with_role_provider("local/qwen3-8b", local).with_ledger(ledger);
    let (out, _) = run("/run 2", &store, &runner).await.unwrap();
    // the summary role found the server gone; the digest and the classification made no call
    // after it, and none of the three went to the card's model
    assert_eq!(reached.load(Ordering::SeqCst), 1, "{out:?}");
    assert_eq!(card.seen.lock().len(), 2);
    assert_eq!(runner.tally().calls, 2);
    assert_eq!(runner.by_model().await.unwrap().len(), 1);
    // each role's empty result: the handoff carries the whole answer, the bullet stays untagged
    // and the digest line is the feed's end line
    let task = ra_context::TaskContext::open(dir.path(), 2);
    let report = &task.handoffs.last().unwrap().report;
    assert!(report.starts_with("wrote the greeting into hello.txt") && report.ends_with("grep in Bash works"), "{report}");
    assert!(out.iter().any(|l| l.contains("↯ Select-String")), "{out:?}");
    assert!(!crate::retro::recent(dir.path(), 1)[0].machine);
    assert!(crate::hub::own_digest(dir.path()).unwrap().last.unwrap().starts_with("✓ #2 Probe implemented"));
}

#[tokio::test]
async fn a_notes_for_line_lands_on_the_other_card_and_leaves_this_cards_notes() {
    let dir = tempfile::tempdir().unwrap();
    cargo_project(dir.path(), true);
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    run("/add task Next parent 1", &store, &worker()).await.unwrap();
    let report = "wrote hello.txt\nnotes-for #3: the greeting lives in hello.txt\nnotes-for #99: nobody\n";
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(Scripted::new(vec![says(report, vec![]), says("accepted", vec![])]));
    let (out, _) = run("/run 2", &store, &runner).await.unwrap();
    // the card's own notes and handoff lose the lines
    assert_eq!(store.read(2).await.unwrap().notes.as_deref(), Some("wrote hello.txt\n"));
    assert_eq!(ra_context::TaskContext::open(dir.path(), 2).handoffs.last().unwrap().report, "wrote hello.txt");
    // the other card got a handoff from this one; the unknown card got a line instead
    let next = ra_context::TaskContext::open(dir.path(), 3);
    let note = next.handoffs.last().unwrap();
    assert_eq!(note.outcome, "note from #2");
    assert_eq!(note.report, "the greeting lives in hello.txt");
    assert!(out.iter().any(|l| l.contains("notes-for #3: left on its context")), "{out:?}");
    assert!(out.iter().any(|l| l.contains("notes-for #99 dropped")), "{out:?}");
}

#[tokio::test]
async fn a_failing_check_leaves_the_node_needing_revision() {
    let (out, node) = checked_run(false).await;
    assert_eq!(node.state, ra_roadmap::model::TaskState::NeedsRevision);
    let notes = node.notes.unwrap_or_default();
    // the failed test leads the notes, the answer follows the output
    assert!(notes.starts_with("check failed: cargo test --workspace -q\nprobe --- FAILED\n"), "{notes}");
    assert!(notes.ends_with("\n\ndone"), "{notes}");
    assert!(out.iter().any(|l| l.contains("check failed: cargo test --workspace -q · 1 test(s) failed")), "{out:?}");
}

#[tokio::test]
async fn a_passing_check_leaves_the_node_implemented() {
    let (out, node) = checked_run(true).await;
    assert_eq!(node.state, ra_roadmap::model::TaskState::Implemented);
    assert_eq!(node.notes.as_deref(), Some("done"));
    assert!(out.iter().any(|l| l.contains("check passed: cargo test --workspace -q")), "{out:?}");
}

#[tokio::test]
async fn a_report_that_says_its_check_is_running_is_overwritten_by_the_agents_own_run_of_it() {
    let dir = tempfile::tempdir().unwrap();
    cargo_project(dir.path(), true);
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    run("/edit 2 acceptance `cargo test -p ra-console -q` passes", &store, &worker()).await.unwrap();
    // the agent runs the check itself, after the worker's last turn, and waits for it
    let seen = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    let log = seen.clone();
    let script: ra_tools::Script = std::sync::Arc::new(move |command: &str| {
        log.lock().push(command.to_owned());
        Ok((0, "running 3 tests\n...\ntest result: ok. 3 passed\n".to_owned()))
    });
    // the worker went on without its check: a background shell the session outlived (#159)
    let report = "wrote hello.txt\ncheck running: `cargo test -p ra-console -q` in a background shell, result pending\nsee the handoff";
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(Scripted::new(vec![says(report, vec![])])).with_script(script);
    let (out, _) = run("/run 2", &store, &worker).await.unwrap();
    assert_eq!(*seen.lock(), ["cargo test -p ra-console -q"]);
    assert!(out.iter().any(|l| l.contains("check passed: cargo test -p ra-console -q")), "{out:?}");
    // the handoff and the notes carry the agent's run, its exit and clipped tail, where the
    // claim was; the claim itself is gone
    let ran = "check passed: cargo test -p ra-console -q · exit code 0\nrunning 3 tests\n...\ntest result: ok. 3 passed\nexit code 0\n\nwrote hello.txt\nsee the handoff";
    let task = ra_context::TaskContext::open(dir.path(), 2);
    assert_eq!(task.handoffs.last().unwrap().report, ran);
    assert_eq!(store.read(2).await.unwrap().notes.as_deref(), Some(ran));
}

/// A run over a git-tracked cargo project whose worker writes `src/extra.rs`; the check and the
/// lint come from a script, with clippy warning at `at`. The node after the run, and the lines.
async fn linted_run(at: &str) -> (Vec<String>, ra_roadmap::model::Node, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    cargo_project(dir.path(), true);
    // the project's own files are in the index before the run, so the handoff names only what
    // the worker wrote
    assert!(std::process::Command::new("git").args(["init", "-q"]).current_dir(dir.path()).status().unwrap().success());
    assert!(std::process::Command::new("git").args(["add", "-A"]).current_dir(dir.path()).status().unwrap().success());
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    let write = ra_providers::ToolCall {
        id: "call_1".into(),
        name: "write_file".into(),
        arguments: r#"{"path":"src/extra.rs","content":"pub fn extra() { let x = 1; }\n"}"#.into(),
    };
    let clippy = format!(
        "    Checking probe v0.1.0\nwarning: unused variable: `x`\n --> {at}:1:22\n  |\n1 | pub fn extra() {{ let x = 1; }}\n  |                      ^\n\nwarning: `probe` (lib) generated 1 warning\n"
    );
    let seen = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    let log = seen.clone();
    let script: ra_tools::Script = std::sync::Arc::new(move |command: &str| {
        log.lock().push(command.to_owned());
        Ok((0, if command.starts_with("cargo clippy") { clippy.clone() } else { "ok\n".to_owned() }))
    });
    let provider = Scripted::new(vec![says("", vec![write]), says("added extra", vec![]), says("accepted", vec![])]);
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(provider).with_script(script);
    let (out, _) = run("/run 2", &store, &worker).await.unwrap();
    // the check, then the lines the change touched (the script's "ok" is no diff, so the new
    // file counts whole), then the lint of the crate of the change
    let lint = format!("cargo clippy -p {}", probe_name(dir.path()));
    assert_eq!(*seen.lock(), ["cargo test --workspace -q", "git diff -U0 HEAD -- src/extra.rs", lint.as_str()]);
    (out, store.read(2).await.unwrap(), dir)
}

#[tokio::test]
async fn a_warning_in_a_changed_file_sends_the_card_to_needs_revision() {
    let (out, node, dir) = linted_run("src/extra.rs").await;
    assert_eq!(node.state, ra_roadmap::model::TaskState::NeedsRevision);
    let notes = node.notes.unwrap_or_default();
    assert!(notes.starts_with("warning: unused variable: `x`\n --> src/extra.rs:1:22"), "the warning comes first: {notes}");
    assert!(notes.ends_with("\n\nadded extra"), "{notes}");
    assert!(out.iter().any(|l| l.contains("lint: 1 warning(s)") && l.contains("cargo clippy -p probe")), "{out:?}");
    let context = std::fs::read_to_string(dir.path().join("var/context/2.md")).unwrap();
    let handoff = context.split("## Handoff ").nth(1).unwrap();
    assert!(handoff.contains("needs revision") && handoff.contains("changed: src/extra.rs"), "{handoff}");
    assert!(handoff.contains("warning: unused variable: `x`"), "the handoff carries the warning: {handoff}");
    // under its own heading, after the report, not at the head of the report
    assert!(handoff.contains("\nadded extra\n### Warnings\nwarning: unused variable: `x`\n --> src/extra.rs:1:22\n"), "{handoff}");
    // the next run of the card gets the warning in its prompt and, on a harness, in its machine
    // notes as a system message
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    let ok: ra_tools::Script = std::sync::Arc::new(|_: &str| Ok((0, "ok\n".to_owned())));
    let provider = Scripted::new(vec![says("fixed", vec![]), says("accepted", vec![])]);
    let worker = Worker::new("claude-fable-5-1").with_provider(std::sync::Arc::new(Harness(provider.clone()))).with_script(ok.clone());
    run("/run 2", &store, &worker).await.unwrap();
    // the run's request, not the reviewer's after it
    let request = provider.seen.lock().remove(0);
    let note = request.messages.iter().find(|m| m.role == "system" && m.content.contains("Fix these lines first")).expect("the note");
    assert!(note.content.contains("left 1 warning(s)") && note.content.contains("--> src/extra.rs:1:22"), "{}", note.content);
    // that run changed nothing and came out clean: the note is gone, the handoff before still
    // shows the lines in the prompt of a bare model
    let provider = Scripted::new(vec![says("fixed", vec![]), says("accepted", vec![])]);
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(provider.clone()).with_script(ok);
    run("/run 2", &store, &worker).await.unwrap();
    let request = provider.seen.lock().remove(0);
    assert!(!request.messages.iter().any(|m| m.content.contains("Fix these lines first")), "{:?}", request.messages);
    let prompt = &request.messages.last().unwrap().content;
    assert!(prompt.contains("### Warnings\nwarning: unused variable: `x`\n --> src/extra.rs:1:22"), "{prompt}");
}

/// [`linted_run`] in a project whose `[role]` table puts the fix pass on the scripted model: the
/// first clippy warns in `src/extra.rs`, and the next one is clean when `clean_after`. The
/// lines, the node after the run, the commands the scripts ran and the model's requests.
async fn lint_fixed_run(
    clean_after: bool,
) -> (Vec<String>, ra_roadmap::model::Node, Vec<String>, std::sync::Arc<Scripted>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    cargo_project(dir.path(), true);
    std::fs::write(dir.path().join(crate::router::ROUTES_FILE), "[role]\nfix = \"z-ai/glm-5.3-flash\"\n").unwrap();
    assert!(std::process::Command::new("git").args(["init", "-q"]).current_dir(dir.path()).status().unwrap().success());
    assert!(std::process::Command::new("git").args(["add", "-A"]).current_dir(dir.path()).status().unwrap().success());
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    let write = ra_providers::ToolCall {
        id: "call_1".into(),
        name: "write_file".into(),
        arguments: r#"{"path":"src/extra.rs","content":"pub fn extra() { let x = 1; }\n"}"#.into(),
    };
    let clippy = "    Checking probe v0.1.0\nwarning: unused variable: `x`\n --> src/extra.rs:1:22\n  |\n1 | pub fn extra() { let x = 1; }\n  |                      ^\n\nwarning: `probe` (lib) generated 1 warning\n";
    let seen = std::sync::Arc::new(parking_lot::Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    let script: ra_tools::Script = std::sync::Arc::new(move |command: &str| {
        let mut log = log.lock();
        let lints = log.iter().filter(|c| c.starts_with("cargo clippy")).count();
        log.push(command.to_owned());
        let warns = command.starts_with("cargo clippy") && (lints == 0 || !clean_after);
        Ok((0, if warns { clippy.to_owned() } else { "ok\n".to_owned() }))
    });
    let mut answers = vec![says("", vec![write]), says("added extra", vec![]), says("dropped the unused x", vec![])];
    if clean_after {
        answers.push(says("accepted", vec![]));
    }
    let model = Scripted::new(answers);
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(model.clone()).with_script(script).with_debug(false);
    let (out, _) = run("/run 2", &store, &worker).await.unwrap();
    let commands = seen.lock().clone();
    (out, store.read(2).await.unwrap(), commands, model, dir)
}

#[tokio::test]
async fn a_lint_only_failure_takes_the_cheap_fix_pass_and_goes_on_to_the_reviewer() {
    let (out, node, commands, model, dir) = lint_fixed_run(true).await;
    assert_eq!(node.state, ra_roadmap::model::TaskState::Implemented, "{out:?}");
    // the end line is the pass's, with the note; the first run's lines follow, the verdict last
    assert!(out[0].starts_with("✓ #2 Probe implemented on glm-5.3-flash"), "{out:?}");
    assert!(out[0].ends_with(" · lint fixed on glm-5.3-flash"), "{out:?}");
    let clean = out.iter().position(|l| l.contains("lint clean: cargo clippy -p probe")).expect("the pass's lint line");
    let before = out.iter().position(|l| l == "  the run before the lint fix pass on glm-5.3-flash:").expect("the pass line");
    let first = out.iter().position(|l| l.starts_with("✗ #2 Probe needs revision on glm-5.3-flash")).expect("the first run's line");
    let verdict = out.iter().position(|l| l.starts_with("✓ #2 Probe accepted by glm-5.3-flash")).expect("the verdict");
    assert!(clean < before && before < first && first < verdict, "{out:?}");
    assert!(!out.iter().any(|l| l.contains("waits for the person")), "{out:?}");
    // the pass ran the check and the lint again
    assert_eq!(commands.iter().filter(|c| c.starts_with("cargo clippy")).count(), 2, "{commands:?}");
    assert_eq!(commands.iter().filter(|c| c.starts_with("cargo test")).count(), 2, "{commands:?}");
    // the warning lines led the pass's task; the reviewer came after
    let seen = model.seen.lock();
    assert_eq!(seen.len(), 4);
    let prompt = &seen[2].messages.last().unwrap().content;
    assert!(prompt.contains("The lint of this card's run left the warnings below"), "{prompt}");
    assert!(prompt.contains("warning: unused variable: `x`\n --> src/extra.rs:1:22"), "{prompt}");
    assert!(seen[3].tools.is_empty(), "the verdict is a review");
    // the handoffs: the run with its warnings, the pass without, the verdict on the pass's
    let handoffs = ra_context::TaskContext::open(dir.path(), 2).handoffs;
    assert_eq!(handoffs.len(), 2);
    assert_eq!(handoffs[0].outcome, "needs revision");
    assert_eq!(handoffs[0].warnings.len(), 1);
    assert_eq!(handoffs[1].outcome, "implemented");
    assert!(handoffs[1].warnings.is_empty());
    assert!(handoffs[1].verdict.as_deref().is_some_and(|v| v.starts_with("accepted")), "{:?}", handoffs[1].verdict);
}

#[tokio::test]
async fn a_lint_still_red_after_the_fix_pass_leaves_the_card_needs_revision() {
    let (out, node, commands, model, dir) = lint_fixed_run(false).await;
    assert_eq!(node.state, ra_roadmap::model::TaskState::NeedsRevision, "{out:?}");
    assert!(out[0].starts_with("✗ #2 Probe needs revision on glm-5.3-flash"), "{out:?}");
    assert!(out.iter().any(|l| l == "  lint fix pass on glm-5.3-flash"), "{out:?}");
    assert!(!out.iter().any(|l| l.contains("lint fixed on")), "{out:?}");
    assert_eq!(out.iter().filter(|l| l.contains("lint: 1 warning(s)")).count(), 2, "{out:?}");
    // one pass, no reviewer, no third run
    assert_eq!(commands.iter().filter(|c| c.starts_with("cargo clippy")).count(), 2, "{commands:?}");
    assert_eq!(model.seen.lock().len(), 3);
    let handoffs = ra_context::TaskContext::open(dir.path(), 2).handoffs;
    assert_eq!(handoffs.len(), 2);
    assert!(handoffs.iter().all(|h| h.outcome == "needs revision" && h.warnings.len() == 1));
    assert!(node.notes.unwrap_or_default().starts_with("warning: unused variable: `x`"));
}

#[tokio::test]
async fn a_warning_elsewhere_leaves_the_card_implemented() {
    let (out, node, _dir) = linted_run("src/lib.rs").await;
    assert_eq!(node.state, ra_roadmap::model::TaskState::Implemented);
    assert_eq!(node.notes.as_deref(), Some("added extra"));
    assert!(out.iter().any(|l| l.contains("lint clean: cargo clippy -p probe")), "{out:?}");
}

/// A run of card #2 in `dir`, whose worker writes `src/extra.rs`; the commands the checks ran
/// and the lines of the run.
async fn run_writing_extra(dir: &std::path::Path) -> (Vec<String>, Vec<String>) {
    let store = Store::open(&dir.join("rhipper.md"), "Rhipper").await.unwrap();
    let write = ra_providers::ToolCall {
        id: "call_1".into(),
        name: "write_file".into(),
        arguments: r#"{"path":"src/extra.rs","content":"pub fn extra() {}\n"}"#.into(),
    };
    let seen = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    let log = seen.clone();
    let script: ra_tools::Script = std::sync::Arc::new(move |command: &str| {
        log.lock().push(command.to_owned());
        Ok((0, "ok\n".to_owned()))
    });
    let provider = Scripted::new(vec![says("", vec![write]), says("added extra", vec![]), says("accepted", vec![])]);
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(provider).with_script(script);
    let (out, _) = run("/run 2", &store, &worker).await.unwrap();
    let seen = seen.lock().clone();
    (seen, out)
}

#[tokio::test]
async fn the_projects_check_stands_in_for_the_workspace_tests_and_an_empty_lint_skips_it() {
    let dir = tempfile::tempdir().unwrap();
    cargo_project(dir.path(), true);
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    // before the table: the workspace tests, and clippy over what changed
    let (out, _) = run("/run 2 dry", &store, &worker()).await.unwrap();
    assert!(out.iter().any(|l| l.contains("check: cargo test --workspace -q · lint: clippy over the changed crates")), "{out:?}");
    std::fs::write(dir.path().join(crate::router::ROUTES_FILE), "[worker]\ncheck = \".\\\\gw.ps1 :m:task\"\nlint = \"\"\n").unwrap();
    let (out, _) = run("/run 2 dry", &store, &worker()).await.unwrap();
    assert!(out.iter().any(|l| l.contains("check: .\\gw.ps1 :m:task · lint: none")), "{out:?}");
    // a card without a backticked command runs the project's check and nothing lints the change
    let (seen, out) = run_writing_extra(dir.path()).await;
    assert_eq!(seen, [".\\gw.ps1 :m:task"]);
    assert!(out.iter().any(|l| l.contains("check passed: .\\gw.ps1 :m:task")), "{out:?}");
    assert!(!out.iter().any(|l| l.contains("lint")), "{out:?}");
    assert_eq!(store.read(2).await.unwrap().state, ra_roadmap::model::TaskState::Implemented);
    let pair = crate::pair::recent(dir.path(), 10);
    assert!(pair.iter().any(|l| l.starts_with("start #2 Probe") && l.contains("check: .\\gw.ps1 :m:task · lint: none · pid ")), "{pair:?}");
    // a card with its own backticked check still wins
    run("/edit 2 acceptance `cargo test -q` passes", &store, &worker()).await.unwrap();
    let (seen, out) = run_writing_extra(dir.path()).await;
    assert_eq!(seen, ["cargo test -q"]);
    assert!(out.iter().any(|l| l.contains("check passed: cargo test -q")), "{out:?}");
}

#[tokio::test]
async fn the_projects_check_timeout_shows_on_the_dry_run_and_bounds_the_checks() {
    let dir = tempfile::tempdir().unwrap();
    cargo_project(dir.path(), true);
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    // without the key the agent's twenty minutes stand, and the line does not say so
    let (out, _) = run("/run 2 dry", &store, &worker()).await.unwrap();
    assert!(out.iter().any(|l| l.contains("check: cargo test --workspace -q · lint: clippy over the changed crates")), "{out:?}");
    std::fs::write(
        dir.path().join(crate::router::ROUTES_FILE),
        "[worker]\ncheck = \".\\\\gw.ps1 :m:task\"\nlint = \"\"\ncheck_timeout = 1800\n",
    )
    .unwrap();
    let (out, _) = run("/run 2 dry", &store, &worker()).await.unwrap();
    assert!(out.iter().any(|l| l.contains("check: .\\gw.ps1 :m:task · 1800 s · lint: none")), "{out:?}");
    // the check runs under the project's limit and passes; the start line carries the limit
    let (seen, out) = run_writing_extra(dir.path()).await;
    assert_eq!(seen, [".\\gw.ps1 :m:task"]);
    assert!(out.iter().any(|l| l.contains("check passed: .\\gw.ps1 :m:task")), "{out:?}");
    let pair = crate::pair::recent(dir.path(), 10);
    assert!(
        pair.iter().any(|l| l.starts_with("start #2 Probe") && l.contains("check: .\\gw.ps1 :m:task · 1800 s · lint: none")),
        "{pair:?}"
    );
}

#[tokio::test]
async fn a_check_that_outlives_its_budget_is_killed() {
    let dir = tempfile::tempdir().unwrap();
    cargo_project(dir.path(), true);
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    // a real process, so the limit has something to kill: the sleep outlives the budget by far
    run(r#"/edit 2 acceptance `python -c "print('started', flush=True); import time; time.sleep(60)"` passes"#, &store, &worker())
        .await
        .unwrap();
    std::fs::write(dir.path().join(crate::router::ROUTES_FILE), "[worker]\nlint = \"\"\ncheck_timeout = 1\n").unwrap();
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(Scripted::new(vec![says("done", vec![])]));
    let started = std::time::Instant::now();
    let (out, _) = run("/run 2", &store, &worker).await.unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(30), "the walk did not wait for the sleep: {out:?}");
    assert!(out.iter().any(|l| l.contains("check failed: python -c") && l.contains(" · timed out after 1s")), "{out:?}");
    let node = store.read(2).await.unwrap();
    assert_eq!(node.state, ra_roadmap::model::TaskState::NeedsRevision);
    // python ends its line the platform's way; the output before the limit and the line stay
    let notes = node.notes.unwrap_or_default().replace("\r\n", "\n");
    assert!(notes.starts_with("check failed: python -c"), "{notes}");
    assert!(notes.contains("started\ntimed out after 1s\n\ndone"), "{notes}");
    let task = ra_context::TaskContext::open(dir.path(), 2);
    assert!(task.handoffs.last().is_some_and(|h| h.report.contains("timed out after 1s")), "{:?}", task.handoffs.last());
}

#[tokio::test]
async fn a_long_check_output_is_clipped() {
    let dir = tempfile::tempdir().unwrap();
    cargo_project(dir.path(), true);
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    run("/edit 2 acceptance `cargo test -q` passes", &store, &worker()).await.unwrap();
    // three hundred lines with two failures early on: only the first leads the tail
    let mut lines: Vec<String> = (0..300).map(|i| format!("test a_passing_test_{i} ... ok")).collect();
    lines[10] = "test the_first_broken ... FAILED".to_owned();
    lines[20] = "test the_second_broken ... FAILED".to_owned();
    let output = format!("{}\n", lines.join("\n"));
    let script: ra_tools::Script = std::sync::Arc::new(move |_: &str| Ok((101, output.clone())));
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(Scripted::new(vec![says("done", vec![])])).with_script(script);
    let (out, _) = run("/run 2", &store, &worker).await.unwrap();
    assert!(out.iter().any(|l| l.contains("check failed: cargo test -q · 2 test(s) failed")), "{out:?}");
    let notes = store.read(2).await.unwrap().notes.unwrap_or_default();
    let kept: Vec<&str> = notes.lines().collect();
    assert_eq!(kept[0], "check failed: cargo test -q");
    assert_eq!(kept[1], "test the_first_broken ... FAILED");
    assert_eq!(kept[2], "");
    // the tail: sixty lines of output, the exit line last, then the worker's answer
    assert_eq!(kept[3], "test a_passing_test_241 ... ok");
    assert_eq!(kept[3 + 59], "exit code 101");
    assert_eq!(&kept[kept.len() - 2..], ["", "done"]);
    assert!(!notes.contains("the_second_broken"), "{notes}");
    assert!(!notes.contains("a_passing_test_240 "), "{notes}");
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
    let (out, _) = run("/reassess 2", &store, &worker).await.unwrap();
    assert!(out[0].contains("0 unchanged"), "{out:?}");
    let assessed = store.read(2).await.unwrap().assessed.unwrap();
    assert_eq!(assessed.len(), 16, "{assessed}");
    assert!(assessed.chars().all(|c| c.is_ascii_hexdigit()), "{assessed}");

    let (out, _) = run("/reassess 2", &store, &worker).await.unwrap();
    assert_eq!(provider.seen.lock().len(), 1, "the second pass made no call");
    assert!(out[0].contains("1 unchanged"), "{out:?}");
    assert!(out.iter().any(|l| l.contains("#2 T assessed and unchanged")), "{out:?}");
    assert!(!out.iter().any(|l| l.starts_with('⚠')), "{out:?}");
}

#[tokio::test]
async fn a_final_node_is_skipped_with_no_call() {
    let store = store_with_a_task().await;
    let (out, _) = run("/final 2", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["#2 is final: no reassessment rewrites it"]);
    let (out, _) = run("/show", &store, &worker()).await.unwrap();
    assert!(out.iter().any(|l| l.contains("#2 T [defined] [final]")), "{out:?}");

    let provider = Scripted::new(Vec::new());
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(provider.clone());
    // the project above still calls, and the empty script turns that into a warning line
    let (out, _) = run("/reassess-tree 1", &store, &worker).await.unwrap();
    assert!(provider.seen.lock().iter().all(|r| !r.messages.iter().any(|m| m.content.contains("Task #2:"))), "#2 made no call");
    assert!(out.iter().any(|l| l.contains("#2 T [final] assessed and unchanged")), "{out:?}");
    assert_eq!(store.read(2).await.unwrap().objective.as_deref(), Some("do it."));

    let (out, _) = run("/final 2 off", &store, &worker).await.unwrap();
    assert_eq!(out, vec!["#2 is no longer final"]);
    assert!(store.read(2).await.unwrap().assessed.is_none());
    let (out, _) = run("/show", &store, &worker).await.unwrap();
    assert!(!out.iter().any(|l| l.contains("[final]")), "{out:?}");
}

#[tokio::test]
async fn a_final_single_node_makes_no_call() {
    let store = store_with_a_task().await;
    run("/final 2", &store, &worker()).await.unwrap();
    let provider = Scripted::new(Vec::new());
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(provider.clone());
    let (out, _) = run("/reassess 2", &store, &worker).await.unwrap();
    assert!(provider.seen.lock().is_empty(), "no provider call");
    assert!(out[0].contains("1 unchanged"), "{out:?}");
    assert!(store.read(2).await.unwrap().is_final(), "the mark stays");
}

#[tokio::test]
async fn editing_the_objective_after_an_assessment_calls_again() {
    let store = store_with_a_task().await;
    let provider = Scripted::new(vec![keeps("do it.", "it is done."), keeps("do it twice.", "it is done twice.")]);
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(provider.clone());
    run("/reassess 2", &store, &worker).await.unwrap();
    let first = store.read(2).await.unwrap().assessed.unwrap();
    store.set_details(2, Some("do it twice."), None).await.unwrap();

    let (out, _) = run("/reassess 2", &store, &worker).await.unwrap();
    assert_eq!(provider.seen.lock().len(), 2, "the edit made the next pass call");
    assert!(out[0].contains("0 unchanged"), "{out:?}");
    let node = store.read(2).await.unwrap();
    assert_eq!(node.acceptance.as_deref(), Some("it is done twice."));
    assert_ne!(node.assessed.unwrap(), first, "the fingerprint follows the new text");
}

/// The answer of a milestone's reassessment that keeps the text and proposes two cards.
fn proposes_two() -> ra_providers::Completion {
    says(
        "overview: two cards are missing\nobjective: ship it.\nacceptance: it ships.\n\
         card: Login | users sign in | `cargo test -p auth` passes\ncard: Logout | users sign out | the session ends",
        Vec::new(),
    )
}

#[tokio::test]
async fn a_reassess_tree_proposes_cards_the_person_accepts_or_declines() {
    let store = store().await;
    store.insert(Some(1), ra_roadmap::model::NodeKind::Milestone, "M", Some("ship it."), Some("it ships."), 3).await.unwrap();
    // the project calls first and keeps its text; the milestone proposes
    let provider = Scripted::new(vec![keeps("the agent.", "it works."), proposes_two()]);
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(provider.clone());
    let (out, _) = run("/reassess-tree 1", &store, &worker).await.unwrap();
    let list = out.iter().find(|l| l.contains("2 card(s) proposed under #2")).expect("the list");
    assert!(list.contains("\n  1. Login — users sign in\n  2. Logout — users sign out"), "{list}");
    let (shown, _) = run("/proposals 2", &store, &worker).await.unwrap();
    assert_eq!(shown.len(), 3, "{shown:?}");

    let (out, _) = run("/accept 2 1", &store, &worker).await.unwrap();
    assert_eq!(out[0], "#3 Login added under #2");
    assert!(out[1].contains("1 card(s) proposed"), "{out:?}");
    let login = store.read(3).await.unwrap();
    assert_eq!((login.parent, login.objective.as_deref()), (Some(2), Some("users sign in")));
    assert_eq!(login.acceptance.as_deref(), Some("`cargo test -p auth` passes"));

    let (out, _) = run("/decline 2 1", &store, &worker).await.unwrap();
    assert_eq!(out, vec!["declined Logout: it does not return"]);
    let assessed = store.read(2).await.unwrap().assessed.unwrap();
    let words: Vec<&str> = assessed.split_whitespace().collect();
    assert_eq!(words.len(), 2, "the fingerprint and one declined mark: {assessed}");
    assert!(words[1].starts_with('-') && words[1].len() == 17, "{assessed}");
    assert!(agent::proposals_of(&store, 2).is_empty());
    assert!(run("/accept 2 1", &store, &worker).await.is_err(), "nothing waits");

    // the next pass proposes the same two: one is a child now, the other was declined; the
    // child is pinned so the milestone is the only call
    run("/final 3", &store, &worker).await.unwrap();
    let provider = Scripted::new(vec![proposes_two()]);
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(provider.clone());
    let (out, _) = run("/reassess-tree 2", &store, &worker).await.unwrap();
    assert_eq!(provider.seen.lock().len(), 1, "the new child changed the milestone's fingerprint");
    assert!(!out.iter().any(|l| l.contains("proposed")), "{out:?}");
    assert!(store.read(2).await.unwrap().assessed.unwrap().contains(words[1]), "the declined mark stays");
}

#[tokio::test]
async fn a_reassessment_proposes_parking_a_card_no_gambit_covers_and_the_person_decides() {
    let store = store().await;
    store.insert(Some(1), ra_roadmap::model::NodeKind::Milestone, "M", Some("ship it."), Some("it ships."), 3).await.unwrap();
    store.insert(Some(2), ra_roadmap::model::NodeKind::Task, "T", Some("do it."), Some("it is done."), 2).await.unwrap();
    store.insert(Some(2), ra_roadmap::model::NodeKind::Task, "Local cache", Some("cache it."), Some("it caches."), 2).await.unwrap();
    store.add_gambit("Local first", 3, &["local".to_owned()]).await.unwrap();
    // the project, the milestone, then its two children, parent first; the milestone's answer
    // keeps its text and asks to park #3, the one card no direction covers
    let parks =
        says("overview: T serves no direction\nobjective: ship it.\nacceptance: it ships.\npark: #3 | nothing local about it", Vec::new());
    let provider =
        Scripted::new(vec![keeps("the agent.", "it works."), parks, keeps("do it.", "it is done."), keeps("cache it.", "it caches.")]);
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(provider.clone());
    let (out, _) = run("/reassess-tree 1", &store, &worker).await.unwrap();
    // the prompts carry the direction, and the milestone's alone offers the park
    let seen = provider.seen.lock();
    assert!(seen.iter().all(|r| r.messages.iter().any(|m| m.content.contains("- g1 Local first {w3} · focus: local"))), "{seen:?}");
    let milestone = seen.iter().find(|r| r.messages.iter().any(|m| m.content.contains("Task #2: M"))).expect("the milestone's call");
    assert!(milestone.messages.iter().any(|m| m.content.contains("park: #<its id>")), "{milestone:?}");
    assert!(!seen[0].messages.iter().any(|m| m.content.contains("park: #<its id>")), "the project is not asked");
    drop(seen);
    let list = out.iter().find(|l| l.contains("1 card(s) proposed under #2")).expect("the list");
    assert!(list.contains("\n  1. park #3 T — nothing local about it"), "{list}");
    // shown, not applied: #3 is as it was until the person says so
    assert_eq!(store.read(3).await.unwrap().state, ra_roadmap::model::TaskState::Defined);
    let (shown, _) = run("/proposals 2", &store, &worker).await.unwrap();
    assert!(shown[1].contains("park #3 T"), "{shown:?}");

    let (out, _) = run("/accept 2 1", &store, &worker).await.unwrap();
    assert!(out[0].starts_with("#3 T parked under #2: blocked on your word"), "{out:?}");
    let parked = store.read(3).await.unwrap();
    assert_eq!(parked.state, ra_roadmap::model::TaskState::Blocked);
    assert_eq!(parked.notes.as_deref(), Some("parked on the person's word: nothing local about it"));
    assert_eq!(store.read(4).await.unwrap().state, ra_roadmap::model::TaskState::Defined);
}

#[tokio::test]
async fn a_declined_park_keeps_the_card_and_is_not_proposed_again() {
    let store = store().await;
    store.insert(Some(1), ra_roadmap::model::NodeKind::Milestone, "M", Some("ship it."), Some("it ships."), 3).await.unwrap();
    store.insert(Some(2), ra_roadmap::model::NodeKind::Task, "T", Some("do it."), Some("it is done."), 2).await.unwrap();
    store.add_gambit("Local first", 3, &["local".to_owned()]).await.unwrap();
    run("/final 3", &store, &worker()).await.unwrap();
    let parks = || says("overview: x\nobjective: ship it.\nacceptance: it ships.\npark: #3 | nothing local about it", Vec::new());
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(Scripted::new(vec![parks()]));
    let (out, _) = run("/reassess-tree 2", &store, &worker).await.unwrap();
    assert!(out.iter().any(|l| l.contains("park #3 T")), "{out:?}");
    let (out, _) = run("/decline 2 1", &store, &worker).await.unwrap();
    assert_eq!(out, vec!["declined parking #3 T: it stays, and is not proposed again"]);
    assert_eq!(store.read(3).await.unwrap().state, ra_roadmap::model::TaskState::Defined);
    // the next pass, after an edit, offers the same park: the declined mark keeps it out
    store.set_details(2, Some("ship it all."), None).await.unwrap();
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(Scripted::new(vec![parks()]));
    let (out, _) = run("/reassess-tree 2", &store, &worker).await.unwrap();
    assert!(!out.iter().any(|l| l.contains("proposed")), "{out:?}");
}

#[tokio::test]
async fn run_on_a_bare_model_fails_the_node_when_the_provider_does() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task T parent 1", &store, &worker()).await.unwrap();
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(Scripted::new(Vec::new()));
    let error = run("/run 2", &store, &worker).await.unwrap_err();
    assert!(format!("{error:#}").contains("the script ran out"), "{error:#}");
    assert_eq!(store.read(2).await.unwrap().state, ra_roadmap::model::TaskState::Failed);
}

#[test]
fn the_complexity_sets_the_budget() {
    use crate::agent::budget_for;
    assert_eq!([1, 2, 3, 4, 5].map(budget_for), [0.25, 0.50, 1.00, 2.00, 5.00]);
}

#[test]
fn the_complexity_sets_the_estimate_unless_the_card_names_one() {
    use crate::agent::{estimate_for, estimate_of};
    use ra_roadmap::model::Estimate;
    assert_eq!([1, 2, 3, 4, 5].map(|c| estimate_for(c).tokens), [20_000, 50_000, 120_000, 250_000, 500_000]);
    assert_eq!([1, 2, 3, 4, 5].map(|c| estimate_for(c).turns), [4, 8, 15, 25, 40]);
    assert_eq!(estimate_for(0), estimate_for(1));
    assert_eq!(estimate_for(9), estimate_for(5));
    let mut node = ra_roadmap::markdown::parse("- 1 ‹task› T {c2}").unwrap().remove(0);
    assert_eq!(estimate_of(&node), estimate_for(2));
    node.estimate = Some(Estimate { tokens: 500, turns: 1 });
    assert_eq!(estimate_of(&node), Estimate { tokens: 500, turns: 1 });
}

/// A run of a c3 card whose harness reports `tokens` read evenly over `turns` turns; the lines
/// and the handoff of the run.
async fn sized_run(tokens: u64, turns: u64, estimate: Option<ra_roadmap::model::Estimate>) -> (Vec<String>, String) {
    let dir = tempfile::tempdir().unwrap();
    cargo_project(dir.path(), true);
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Probe parent 1", &store, &worker()).await.unwrap();
    store.set_estimate(2, estimate).await.unwrap();
    let mut answer = says("done", vec![]);
    answer.usage.prompt_tokens = tokens;
    answer.turns = turns;
    let a_turn = ra_providers::Usage { prompt_tokens: tokens / turns, completion_tokens: 10 / turns, ..answer.usage };
    answer.turn_usage = (0..turns).map(|_| ra_providers::TurnUsage::of(&a_turn)).collect();
    let worker = Worker::new("claude-opus-5-5").with_provider(std::sync::Arc::new(Harness(Scripted::new(vec![answer]))));
    let (out, _) = run("/run 2", &store, &worker).await.unwrap();
    let context = std::fs::read_to_string(dir.path().join("var/context/2.md")).unwrap();
    let handoff = context.split("## Handoff ").nth(1).unwrap().to_owned();
    (out, handoff)
}

#[tokio::test]
async fn a_run_whose_turns_carried_more_than_the_estimates_is_over() {
    // a c3 card is 120k tokens over 15 turns, 8k a turn: one turn of 400k is over the measure
    let (out, handoff) = sized_run(400_000, 1, None).await;
    assert!(out[0].contains("over the estimate of 8k a turn at 400k a turn"), "{out:?}");
    assert!(handoff.contains("over the estimate"), "{handoff}");
    // the context a turn carried, on the run line and in the handoff
    assert!(out[0].contains("avg 400k tokens a turn, largest 400k"), "{out:?}");
    assert!(handoff.contains("avg 400k tokens a turn"), "{handoff}");
}

#[tokio::test]
async fn a_run_of_many_lean_turns_is_under_whatever_its_total_and_the_card_can_size_itself() {
    // 100k in all, more turns than the estimate, but 5k a turn against its 8k
    let (out, handoff) = sized_run(100_000, 20, None).await;
    assert!(out[0].contains("under the estimate of 8k a turn at 5k a turn"), "{out:?}");
    assert!(handoff.contains("under the estimate"), "{handoff}");
    // the card's own line is the measure: 110 tokens a turn against 50 is over
    let estimate = ra_roadmap::model::Estimate { tokens: 50, turns: 1 };
    let (out, _) = sized_run(100, 1, Some(estimate)).await;
    assert!(out[0].contains("over the estimate of 50 a turn at 110 a turn"), "{out:?}");
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
    run("/add task T parent 1", &store, &worker()).await.unwrap();
    // a report, not a placeholder: a bare "done" would be asked again and spend the script
    let mut first = says("done: the tree already holds the work", vec![]);
    first.session_id = Some("sess-1".into());
    let script = Scripted::new(vec![first, says("done again", vec![])]);
    let runner = Worker::new("claude-opus-5-5").with_provider(std::sync::Arc::new(Harness(script.clone())));
    run("/run 2", &store, &runner).await.unwrap();
    assert_eq!(store.read(2).await.unwrap().session.as_deref(), Some("sess-1"));
    // the person takes the card up again rather than the check sending it back: not a rerun
    store.set_state(2, ra_roadmap::model::TaskState::InProgress).await.unwrap();
    run("/run 2", &store, &runner).await.unwrap();
    let seen = script.seen.lock();
    assert_eq!(seen[0].resume, None, "a first run has nothing to resume");
    assert_eq!(seen[1].resume.as_deref(), Some("sess-1"), "the second run resumes the first");
}

#[tokio::test]
async fn a_rerun_of_a_card_sent_back_starts_fresh_on_the_verdict_without_the_transcript() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Greet parent 1", &store, &worker()).await.unwrap();
    // the check passes, so the reviewer judges the first run and sends the card back; the fix
    // role is off in the tests, so the next run is a plain rerun, which the reviewer accepts
    let script: ra_tools::Script = std::sync::Arc::new(|_: &str| Ok((0, "ok\nexit code 0\n".to_owned())));
    let provider = Scripted::new(vec![
        says("wrote the greeting into hello.rs and read it back", vec![]),
        says("needs revision\n- rename greet to salute", vec![]),
        says("renamed it", vec![]),
        says("accepted", vec![]),
    ]);
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(provider.clone()).with_script(script);
    run("/run 2", &store, &worker).await.unwrap();
    assert_eq!(store.read(2).await.unwrap().state, ra_roadmap::model::TaskState::NeedsRevision);
    let (out, _) = run("/run 2", &store, &worker).await.unwrap();
    assert!(out.iter().any(|l| l.contains("no session to resume; a rerun carries the verdict")), "{}", out.join("\n"));
    let seen = provider.seen.lock();
    assert_eq!(seen.len(), 4);
    let prompt = &seen[2].messages.last().unwrap().content;
    assert!(prompt.contains("Task #2: Greet"), "{prompt}");
    assert!(prompt.contains("verdict: needs revision (") && prompt.contains("## Review\n"), "{prompt}");
    assert!(
        prompt.contains("\nreview by z-ai/glm-5.3-flash: needs revision (") && prompt.contains(")\n- rename greet to salute\n"),
        "{prompt}"
    );
    assert!(!prompt.contains("wrote the greeting into hello.rs"), "the transcript of the first run went along:\n{prompt}");
    assert!(!prompt.contains("Earlier runs of this task"), "{prompt}");
    assert_eq!(seen[2].resume, None);
    // the file keeps both runs for the person; the rerun did not pay for them
    let task = ra_context::TaskContext::open(dir.path(), 2);
    assert_eq!(task.handoffs.len(), 2, "{:?}", task.handoffs);
}

#[tokio::test]
async fn a_rerun_resumes_the_session_it_came_back_from() {
    // the person: be generous with --resume unless the topic changes. A worker sent back by the
    // reviewer answers the points on the session it read the code in, cache and all; the
    // verdict is its new prompt, the handoffs are not re-sent
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Greet parent 1", &store, &worker()).await.unwrap();
    let script: ra_tools::Script = std::sync::Arc::new(|_: &str| Ok((0, "ok\nexit code 0\n".to_owned())));
    let mut first = says("wrote the greeting into hello.rs", vec![]);
    first.session_id = Some("sess-1".into());
    let mut again = says("renamed it", vec![]);
    again.session_id = Some("sess-1".into());
    let provider = Scripted::new(vec![first, says("needs revision\n- rename greet to salute", vec![]), again, says("accepted", vec![])]);
    let runner = Worker::new("claude-opus-5-5").with_provider(std::sync::Arc::new(Harness(provider.clone()))).with_script(script);
    run("/run 2", &store, &runner).await.unwrap();
    assert_eq!(store.read(2).await.unwrap().state, ra_roadmap::model::TaskState::NeedsRevision);
    let (out, _) = run("/run 2", &store, &runner).await.unwrap();
    assert!(out.iter().any(|l| l.contains("rerun on the session: the verdict")), "{}", out.join("\n"));
    let seen = provider.seen.lock();
    assert_eq!(seen[2].resume.as_deref(), Some("sess-1"), "the rerun resumes the session it came back from");
    let prompt = &seen[2].messages.last().unwrap().content;
    assert!(prompt.contains("## Review\n") && prompt.contains(")\n- rename greet to salute\n"), "{prompt}");
    assert!(!prompt.contains("Earlier runs of this task"), "{prompt}");
}

#[tokio::test]
async fn a_card_sent_back_and_requeued_as_defined_is_still_a_rerun() {
    // rhipper-asm #47: the walk takes only a ready card, so the person re-queued the card with
    // `set defined`; that erased the state the rerun was keyed on, and the worker's prompt
    // lost the five points. The review block at the head of the notes is the key now
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Greet parent 1", &store, &worker()).await.unwrap();
    let script: ra_tools::Script = std::sync::Arc::new(|_: &str| Ok((0, "ok\nexit code 0\n".to_owned())));
    let provider = Scripted::new(vec![
        says("wrote the greeting into hello.rs and read it back", vec![]),
        says("needs revision\n- rename greet to salute", vec![]),
        says("renamed it", vec![]),
        says("accepted", vec![]),
    ]);
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(provider.clone()).with_script(script);
    run("/run 2", &store, &worker).await.unwrap();
    run("/set 2 defined", &store, &worker).await.unwrap();
    assert_eq!(store.read(2).await.unwrap().state, ra_roadmap::model::TaskState::Defined);
    let (out, _) = run("/run 2", &store, &worker).await.unwrap();
    assert!(out.iter().any(|l| l.contains("no session to resume; a rerun carries the verdict")), "{}", out.join("\n"));
    let seen = provider.seen.lock();
    let prompt = &seen[2].messages.last().unwrap().content;
    assert!(prompt.contains("## Review\n") && prompt.contains(")\n- rename greet to salute\n"), "{prompt}");
    assert!(!prompt.contains("wrote the greeting into hello.rs"), "{prompt}");
    assert_eq!(seen[2].resume, None);
}

#[tokio::test]
async fn a_revision_prompt_lists_the_reviewers_points() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task Greet parent 1", &store, &worker()).await.unwrap();
    // the check passes on the first run and the reviewer sends the card back with three points,
    // one of them long; the rerun fails its check, and the run after that reads the points still
    let tests = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = tests.clone();
    let script: ra_tools::Script = std::sync::Arc::new(move |command: &str| {
        if command.starts_with("cargo test") && counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 {
            return Ok((101, "test greet ... FAILED\nexit code 101\n".to_owned()));
        }
        Ok((0, "ok\nexit code 0\n".to_owned()))
    });
    let long = "the greeting is built by hand where format! would do;".repeat(5);
    let provider = Scripted::new(vec![
        says("wrote the greeting into hello.rs", vec![]),
        says(&format!("needs revision\n- rename greet to salute\n- {long}\n- no test covers the empty name"), vec![]),
        says("renamed it", vec![]),
        says("tested it", vec![]),
        says("accepted", vec![]),
    ]);
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(provider.clone()).with_script(script);
    run("/run 2", &store, &worker).await.unwrap();
    run("/run 2", &store, &worker).await.unwrap();
    assert_eq!(store.read(2).await.unwrap().state, ra_roadmap::model::TaskState::NeedsRevision);
    run("/run 2", &store, &worker).await.unwrap();
    let seen = provider.seen.lock();
    assert_eq!(seen.len(), 5);
    let points = format!(")\n- rename greet to salute\n- {long}\n- no test covers the empty name\n");
    // the first rerun reads them from the verdict, the second from the notes the failed rerun
    // left, whole both times and verbatim; neither prompt says only how many there were
    for request in &seen[2..4] {
        let prompt = &request.messages.last().unwrap().content;
        assert!(prompt.contains("## Review\nThe reviewer sent the last run back on these points; answer each:\nreview by z-ai/glm-5.3-flash: needs revision ("), "{prompt}");
        assert!(prompt.contains(&points), "{prompt}");
    }
    assert!(seen[3].messages.last().unwrap().content.contains("check failed: cargo test"), "{}", seen[3].messages.last().unwrap().content);
}

#[test]
fn the_judgment_of_the_notes_is_the_verdict_and_the_failing_check_clipped_and_nothing_after() {
    use crate::agent::judgment_of;
    let notes = "review by m: needs revision (trivial)\n- a point\n\ncheck failed: cargo test -q\ntest a ... FAILED\n\nfailures:\n    a\nexit code 101\n\ncheck passed: python x.py · exit code 0\nok\nexit code 0\n\nthe worker's report";
    let blocks = judgment_of(notes);
    assert_eq!(blocks[0], "review by m: needs revision (trivial)\n- a point");
    assert_eq!(blocks[1], "check failed: cargo test -q\ntest a ... FAILED\n\nfailures:\n    a\nexit code 101");
    assert_eq!(blocks.len(), 2);
    // a long output keeps its head and its tail
    let long: Vec<String> = (0..60).map(|i| format!("line {i}")).collect();
    let notes = format!("check failed: x\n{}\nexit code 1\n\nreport", long.join("\n"));
    let block = &judgment_of(&notes)[0];
    assert!(block.starts_with("check failed: x\nline 0\n") && block.ends_with("line 59\nexit code 1"), "{block}");
    assert!(block.contains("… 32 more lines in the notes of the card"), "{block}");
    assert!(!block.contains("report"));
    assert!(judgment_of("the worker's report\n\ncheck failed: x").is_empty());
}

#[tokio::test]
async fn a_changed_objective_drops_the_resume() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task T parent 1", &store, &worker()).await.unwrap();
    let mut first = says("done: the tree already holds the work", vec![]);
    first.session_id = Some("sess-1".into());
    let script = Scripted::new(vec![first, says("done again", vec![])]);
    let runner = Worker::new("claude-opus-5-5").with_provider(std::sync::Arc::new(Harness(script.clone())));
    run("/run 2", &store, &runner).await.unwrap();
    store.set_details(2, Some("another goal"), None).await.unwrap();
    store.set_state(2, ra_roadmap::model::TaskState::InProgress).await.unwrap();
    let (out, _) = run("/run 2", &store, &runner).await.unwrap();
    let seen = script.seen.lock();
    assert_eq!(seen[1].resume, None, "a changed card starts fresh");
    assert!(out.iter().any(|l| l.contains("fresh start: the card changed since the session")), "{}", out.join("\n"));
}

#[tokio::test]
async fn no_turn_cap_stands_by_default_and_turns_sets_one() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    // nothing caps a run by default; the harness reports the turns it took
    run("/add task T parent 1", &store, &worker()).await.unwrap();
    let mut answer = says("done", vec![]);
    answer.turns = 12;
    let script = Scripted::new(vec![answer]);
    let runner = Worker::new("claude-opus-5-5").with_provider(std::sync::Arc::new(Harness(script.clone())));
    let (out, _) = run("/run 2", &store, &runner).await.unwrap();
    assert!(out[0].contains("turns 12,"), "{out:?}");
    assert_eq!(script.seen.lock()[0].max_turns, None, "nothing caps a run by default");

    // --turns caps one run, and the line names the cap
    run("/add task U parent 1", &store, &worker()).await.unwrap();
    let script = Scripted::new(vec![says("done", vec![])]);
    let runner =
        Worker::new("claude-opus-5-5").with_provider(std::sync::Arc::new(Harness(script.clone()))).with_budgets(false).with_turns(Some(12));
    let (out, _) = run("/run 3", &store, &runner).await.unwrap();
    assert!(out[0].contains("turns 1/12"), "{out:?}");
    let seen = script.seen.lock();
    assert_eq!(seen[0].budget_usd, None);
    assert_eq!(seen[0].max_turns, Some(12));
}

#[tokio::test]
async fn the_effort_follows_the_card_and_the_flag_overrides_it() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task T parent 1", &store, &worker()).await.unwrap();
    // a fresh card runs at medium
    let script = Scripted::new(vec![says("done", vec![])]);
    let runner = Worker::new("claude-opus-5-5").with_provider(std::sync::Arc::new(Harness(script.clone())));
    run("/run 2", &store, &runner).await.unwrap();
    assert_eq!(script.seen.lock()[0].effort.as_deref(), Some("medium"));
    // `effort` sets the card, and the file carries it
    let (out, _) = run("/effort 2 high", &store, &worker()).await.unwrap();
    assert!(out[0].contains("high"), "{out:?}");
    assert_eq!(store.read(2).await.unwrap().effort, ra_roadmap::model::Effort::High);
    let script = Scripted::new(vec![says("done", vec![])]);
    let runner = Worker::new("claude-opus-5-5").with_provider(std::sync::Arc::new(Harness(script.clone())));
    run("/run 2", &store, &runner).await.unwrap();
    assert_eq!(script.seen.lock()[0].effort.as_deref(), Some("high"));
    // --effort overrides the card for one run
    let script = Scripted::new(vec![says("done", vec![])]);
    let runner = Worker::new("claude-opus-5-5")
        .with_provider(std::sync::Arc::new(Harness(script.clone())))
        .with_effort(Some(ra_roadmap::model::Effort::Low));
    run("/run 2", &store, &runner).await.unwrap();
    assert_eq!(script.seen.lock()[0].effort.as_deref(), Some("low"));
    assert!(run("/effort 2 extreme", &store, &worker()).await.is_err());
}

#[tokio::test]
async fn a_bare_model_stops_at_the_turn_cap() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("/add task T parent 1", &store, &worker()).await.unwrap();
    let read = |n: &str| ra_providers::ToolCall { id: n.into(), name: "list_dir".into(), arguments: r#"{"path":"."}"#.into() };
    let provider = Scripted::new(vec![says("looking", vec![read("c1")]), says("still", vec![read("c2")]), says("never reached", vec![])]);
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(provider.clone()).with_turns(Some(2));
    let error = run("/run 2", &store, &runner).await.unwrap_err();
    assert!(format!("{error:#}").contains("2 turns"), "{error:#}");
    assert_eq!(provider.seen.lock().len(), 2);
}

#[tokio::test]
async fn a_run_past_its_budget_stops_and_fails_the_node_with_a_note() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    // with budgets on, a c3 card has a $1.00 budget; off by default, nothing caps the run
    run("/add task T parent 1", &store, &worker()).await.unwrap();
    let read = |n: &str| ra_providers::ToolCall { id: n.into(), name: "list_dir".into(), arguments: r#"{"path":"."}"#.into() };
    let costly = |calls| {
        let mut answer = says("working", calls);
        answer.usage.cost_usd = Some(0.6);
        answer
    };
    let provider = Scripted::new(vec![costly(vec![read("c1")]), costly(vec![read("c2")]), says("never reached", vec![])]);
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(provider.clone()).with_budgets(true);
    let error = run("/run 2", &store, &runner).await.unwrap_err();
    assert!(format!("{error:#}").contains("budget"), "{error:#}");
    assert_eq!(provider.seen.lock().len(), 2, "the loop stopped at the turn that crossed the budget");
    assert_eq!(provider.seen.lock()[0].budget_usd, Some(1.0));
    let node = store.read(2).await.unwrap();
    assert_eq!(node.state, ra_roadmap::model::TaskState::Failed);
    let note = node.notes.unwrap();
    assert!(note.contains("budget") && note.contains("$1.00") && note.contains("$1.2000"), "{note}");

    // --budget overrides the complexity
    run("/add task U parent 1", &store, &worker()).await.unwrap();
    let provider = Scripted::new(vec![says("done", vec![])]);
    let capped = Worker::new("z-ai/glm-5.3-flash").with_provider(provider.clone()).with_budget(Some(0.1));
    run("/run 3", &store, &capped).await.unwrap();
    assert_eq!(provider.seen.lock()[0].budget_usd, Some(0.1));
}

#[test]
fn the_last_run_is_cut_out_of_the_activity_log_by_its_markers() {
    use crate::logs::{RUN_END, RUN_START};
    let activity = format!(
        "{RUN_START}#2 A · opus\nread a.rs\n{RUN_END}✓ $0.01\n{RUN_START}#3 B · opus\nedit b.rs\n{RUN_END}✗ failed\n{RUN_START}#2 A · opus\nread a.rs again\n"
    );
    let newest = last_run_of(&activity, None).unwrap();
    assert_eq!((newest.id, newest.start, newest.lines.len()), (2, 6, 2));
    let of_three = last_run_of(&activity, Some(3)).unwrap();
    assert_eq!(of_three.lines, vec![format!("{RUN_START}#3 B · opus"), "edit b.rs".to_owned(), format!("{RUN_END}✗ failed")]);
    assert_eq!(last_run_of(&activity, Some(2)).unwrap().start, 6);
    assert_eq!(last_run_of(&activity, Some(9)), None);
    assert_eq!(last_run_of("", None), None);
}

#[tokio::test]
async fn log_saves_the_stream_of_a_run_and_names_its_file() {
    use crate::logs::{RUN_END, RUN_START};
    let store = store().await;
    let root = crate::agent::workspace_of(&store);
    std::fs::create_dir_all(root.join("var/runs")).unwrap();
    std::fs::write(
        root.join(crate::ui::ACTIVITY_FILE),
        format!("{RUN_START}#3 B · opus\nedit b.rs\n{RUN_END}✗ failed\n{RUN_START}#2 A · opus\nread a.rs\n{RUN_END}✓ $0.01\n"),
    )
    .unwrap();
    std::fs::write(root.join(ra_providers::claude_code::STREAM_FILE), "{\"type\":\"system\"}\n{\"type\":\"result\"}\n").unwrap();
    let (out, _) = run("/log", &store, &worker()).await.unwrap();
    assert!(out[0].starts_with("#2 · 3 lines · "), "{out:?}");
    let path = std::path::PathBuf::from(out[0].rsplit(" · ").next().unwrap());
    assert!(path.file_name().unwrap().to_str().unwrap().starts_with("2-"), "{out:?}");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.starts_with(&format!("{RUN_START}#2 A")), "{text}");
    assert!(text.contains("--- harness events ---\n{\"type\":\"system\"}\n{\"type\":\"result\"}\n"), "{text}");
    // an older run keeps its progress lines, but the stream on disk is the newer run's
    let (out, _) = run("/log 3", &store, &worker()).await.unwrap();
    assert!(out[0].starts_with("#3 · 3 lines · no harness events"), "{out:?}");
    let text = std::fs::read_to_string(out[0].rsplit(" · ").next().unwrap()).unwrap();
    assert!(text.contains("edit b.rs") && !text.contains("harness events"), "{text}");
    let error = run("/log 9", &store, &worker()).await.unwrap_err();
    assert!(format!("{error:#}").contains("no run of #9"), "{error:#}");
}

#[tokio::test]
async fn run_refuses_a_project_and_names_a_missing_key() {
    let store = store().await;
    let error = run("/run 1", &store, &worker()).await.unwrap_err();
    assert!(format!("{error:#}").contains("is a project"), "{error:#}");
    run("/add task T parent 1", &store, &worker()).await.unwrap();
    // this machine may hold a stored key or a real one in the environment; when neither source
    // holds a key, the run must refuse with the key's name. The env lock covers the read and the
    // removal, so a parallel secret test cannot flip the variable in between.
    let guard = crate::secret::ENV_LOCK.lock();
    let key_configured = std::env::var("OPENROUTER_API_KEY").map_or(false, |k| !k.trim().is_empty()) || crate::secret::has_key();
    if !key_configured {
        // SAFETY: the test holds the env lock; no parallel test reads the key
        unsafe { std::env::remove_var("OPENROUTER_API_KEY") };
        let error = run("/run 2", &store, &worker()).await.unwrap_err();
        assert!(format!("{error:#}").contains("OPENROUTER_API_KEY") || format!("{error:#}").contains("key"), "{error:#}");
    }
    drop(guard);
}
