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
    let error = run("ping", &store().await, &Worker::new("local/x")).await.unwrap_err();
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
    let result = run("ping", &store().await, &worker).await;
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
    assert_eq!(out, vec!["#2 runs on claude-haiku-4-5 through Claude Code"]);
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
    ra_providers::Completion {
        text: text.to_owned(),
        usage,
        turns: 1,
        turn_usage: vec![ra_providers::TurnUsage::of(&usage)],
        tool_calls: calls,
        session_id: None,
        gotchas: Vec::new(),
    }
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
    cargo_project(dir.path(), true);
    let provider =
        Scripted::new(vec![says("", vec![write]), says("checking", vec![read]), says("wrote hello.txt and read it back", vec![])]);
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(provider.clone());

    let (out, _) = run("run 2", &store, &worker).await.unwrap();
    // no cap by default: the line names the turns used alone
    assert!(out[0].contains("turns 3, 2 tool call(s)"), "{out:?}");
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

/// A one-crate Cargo project in `dir` whose single test passes or fails.
fn cargo_project(dir: &std::path::Path, passing: bool) {
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n")
        .unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/lib.rs"), format!("#[test]\nfn probe() {{ assert!({passing}, \"the probe broke\"); }}\n")).unwrap();
}

#[test]
fn the_check_is_the_first_backticked_cargo_command_else_the_workspace_tests() {
    use crate::agent::check_command;
    assert_eq!(check_command(Some("run `rc gotchas` then `cargo test -p ra-console` and `cargo clippy`")), "cargo test -p ra-console");
    assert_eq!(check_command(Some("it is done, see `git log`")), "cargo test --workspace -q");
    assert_eq!(check_command(None), "cargo test --workspace -q");
}

async fn checked_run(passing: bool) -> (Vec<String>, ra_roadmap::model::Node) {
    let dir = tempfile::tempdir().unwrap();
    cargo_project(dir.path(), passing);
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("add task Probe parent 1", &store, &worker()).await.unwrap();
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(Scripted::new(vec![says("done", vec![])]));
    let (out, _) = run("run 2", &store, &worker).await.unwrap();
    let node = store.read(2).await.unwrap();
    (out, node)
}

#[tokio::test]
async fn a_dry_run_prints_the_prompt_and_leaves_the_card_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("add task Probe parent 1", &store, &worker()).await.unwrap();
    let card = store.read(2).await.unwrap();
    store.set_session(2, Some(("sess-1", &crate::agent::card_fingerprint(&card))), crate::agent::unix_now()).await.unwrap();
    let before = store.read(2).await.unwrap();
    // the harness route: the provider builds the command line and no process starts
    let (out, _) = run("run 2 dry", &store, &Worker::new("claude-sonnet-5-5")).await.unwrap();
    let text = out.join("\n");
    assert!(out[0].contains("claude-sonnet-5-5"), "{text}");
    assert!(out.iter().any(|l| l.contains("effort medium") && l.contains("Bash(cargo *)")), "{text}");
    assert!(out.iter().any(|l| l == "  --resume") && out.iter().any(|l| l == "  sess-1"), "{text}");
    assert!(text.contains("--- system ---") && text.contains("--- user ---") && text.contains("Probe"), "{text}");
    let after = store.read(2).await.unwrap();
    assert_eq!(after.state, before.state);
    assert_eq!(after.session.as_deref(), Some("sess-1"));
    assert_eq!(after.notes, before.notes);
    // a bare model: the toolbox is what it would get
    let (out, _) = run("run 2 dry", &store, &worker()).await.unwrap();
    assert!(out.iter().any(|l| l.contains("tools: read_file write_file")), "{}", out.join("\n"));
    assert_eq!(store.read(2).await.unwrap().state, before.state);
}

#[tokio::test]
async fn in_debug_mode_the_run_keeps_the_worker_s_look_back_for_the_next_one() {
    let dir = tempfile::tempdir().unwrap();
    cargo_project(dir.path(), true);
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("add task Probe parent 1", &store, &worker()).await.unwrap();
    let report = "done\n\n## Faster next time\n- [machine] cargo test relinks rc.exe while the window runs: test with -p\n- the card named no files; a files line would have saved the search\n";
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(Scripted::new(vec![says(report, vec![])]));
    assert!(runner.debug, "debug mode is on unless turned off");
    let (out, _) = run("run 2", &store, &runner).await.unwrap();
    assert!(out.iter().any(|l| l.contains("↯ [machine] cargo test relinks")), "{out:?}");
    let kept = crate::retro::recent(dir.path(), 10);
    assert_eq!(kept.len(), 2);
    assert_eq!(kept[0].node, Some(2));
    // the next run reads them as machine notes
    assert!(crate::retro::machine_lines(dir.path()).contains("a files line"));
    // out of debug mode nothing is asked and nothing kept
    run("add task Quiet parent 1", &store, &worker()).await.unwrap();
    let quiet = Worker::new("z-ai/glm-5.3-flash").with_provider(Scripted::new(vec![says(report, vec![])])).with_debug(false);
    let (out, _) = run("run 3", &store, &quiet).await.unwrap();
    assert!(!out.iter().any(|l| l.contains('↯')), "{out:?}");
    assert_eq!(crate::retro::recent(dir.path(), 10).len(), 2);
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
    run("add task Probe parent 1", &store, &worker()).await.unwrap();
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
    let provider = Scripted::new(vec![says("", vec![write]), says("added extra", vec![])]);
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(provider).with_script(script);
    let (out, _) = run("run 2", &store, &worker).await.unwrap();
    // the check, then the lines the change touched (the script's "ok" is no diff, so the new
    // file counts whole), then the lint of the crate of the change
    assert_eq!(*seen.lock(), ["cargo test --workspace -q", "git diff -U0 HEAD -- src/extra.rs", "cargo clippy -p probe"]);
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
    let provider = Scripted::new(vec![says("fixed", vec![])]);
    let worker = Worker::new("claude-fable-5-1").with_provider(std::sync::Arc::new(Harness(provider.clone()))).with_script(ok.clone());
    run("run 2", &store, &worker).await.unwrap();
    let request = provider.seen.lock().pop().unwrap();
    let note = request.messages.iter().find(|m| m.role == "system" && m.content.contains("Fix these lines first")).expect("the note");
    assert!(note.content.contains("left 1 warning(s)") && note.content.contains("--> src/extra.rs:1:22"), "{}", note.content);
    // that run changed nothing and came out clean: the note is gone, the handoff before still
    // shows the lines in the prompt of a bare model
    let provider = Scripted::new(vec![says("fixed", vec![])]);
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(provider.clone()).with_script(ok);
    run("run 2", &store, &worker).await.unwrap();
    let request = provider.seen.lock().pop().unwrap();
    assert!(!request.messages.iter().any(|m| m.content.contains("Fix these lines first")), "{:?}", request.messages);
    let prompt = &request.messages.last().unwrap().content;
    assert!(prompt.contains("### Warnings\nwarning: unused variable: `x`\n --> src/extra.rs:1:22"), "{prompt}");
}

#[tokio::test]
async fn a_warning_elsewhere_leaves_the_card_implemented() {
    let (out, node, _dir) = linted_run("src/lib.rs").await;
    assert_eq!(node.state, ra_roadmap::model::TaskState::Implemented);
    assert_eq!(node.notes.as_deref(), Some("added extra"));
    assert!(out.iter().any(|l| l.contains("lint clean: cargo clippy -p probe")), "{out:?}");
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

/// A run of a c3 card whose scripted provider reports `tokens` read in one turn; the lines and
/// the handoff of the run.
async fn sized_run(tokens: u64, estimate: Option<ra_roadmap::model::Estimate>) -> (Vec<String>, String) {
    let dir = tempfile::tempdir().unwrap();
    cargo_project(dir.path(), true);
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("add task Probe parent 1", &store, &worker()).await.unwrap();
    store.set_estimate(2, estimate).await.unwrap();
    let mut answer = says("done", vec![]);
    answer.usage.prompt_tokens = tokens;
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(Scripted::new(vec![answer]));
    let (out, _) = run("run 2", &store, &worker).await.unwrap();
    let context = std::fs::read_to_string(dir.path().join("var/context/2.md")).unwrap();
    let handoff = context.split("## Handoff ").nth(1).unwrap().to_owned();
    (out, handoff)
}

#[tokio::test]
async fn a_run_that_reports_more_than_its_estimate_says_so() {
    let (out, handoff) = sized_run(400_000, None).await;
    assert!(out[0].contains("over the estimate of 120k tokens, 15 turns"), "{out:?}");
    assert!(handoff.contains("over the estimate"), "{handoff}");
}

#[tokio::test]
async fn a_run_under_its_estimate_says_so_and_the_card_can_size_itself() {
    let (out, handoff) = sized_run(100, None).await;
    assert!(out[0].contains("under the estimate of 120k tokens, 15 turns"), "{out:?}");
    assert!(handoff.contains("under the estimate"), "{handoff}");
    // the card's own line is the measure: 110 tokens against 50 is over
    let estimate = ra_roadmap::model::Estimate { tokens: 50, turns: 1 };
    let (out, _) = sized_run(100, Some(estimate)).await;
    assert!(out[0].contains("over the estimate of 50 tokens, 1 turns"), "{out:?}");
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
async fn a_changed_objective_drops_the_resume() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("add task T parent 1", &store, &worker()).await.unwrap();
    let mut first = says("done", vec![]);
    first.session_id = Some("sess-1".into());
    let script = Scripted::new(vec![first, says("done again", vec![])]);
    let runner = Worker::new("claude-opus-5-5").with_provider(std::sync::Arc::new(Harness(script.clone())));
    run("run 2", &store, &runner).await.unwrap();
    store.set_details(2, Some("another goal"), None).await.unwrap();
    let (out, _) = run("run 2", &store, &runner).await.unwrap();
    let seen = script.seen.lock();
    assert_eq!(seen[1].resume, None, "a changed card starts fresh");
    assert!(out.iter().any(|l| l.contains("fresh start: the card changed since the session")), "{}", out.join("\n"));
}

#[tokio::test]
async fn no_turn_cap_stands_by_default_and_turns_sets_one() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    // nothing caps a run by default; the harness reports the turns it took
    run("add task T parent 1", &store, &worker()).await.unwrap();
    let mut answer = says("done", vec![]);
    answer.turns = 12;
    let script = Scripted::new(vec![answer]);
    let runner = Worker::new("claude-opus-5-5").with_provider(std::sync::Arc::new(Harness(script.clone())));
    let (out, _) = run("run 2", &store, &runner).await.unwrap();
    assert!(out[0].contains("turns 12,"), "{out:?}");
    assert_eq!(script.seen.lock()[0].max_turns, None, "nothing caps a run by default");

    // --turns caps one run, and the line names the cap
    run("add task U parent 1", &store, &worker()).await.unwrap();
    let script = Scripted::new(vec![says("done", vec![])]);
    let runner =
        Worker::new("claude-opus-5-5").with_provider(std::sync::Arc::new(Harness(script.clone()))).with_budgets(false).with_turns(Some(12));
    let (out, _) = run("run 3", &store, &runner).await.unwrap();
    assert!(out[0].contains("turns 1/12"), "{out:?}");
    let seen = script.seen.lock();
    assert_eq!(seen[0].budget_usd, None);
    assert_eq!(seen[0].max_turns, Some(12));
}

#[tokio::test]
async fn the_effort_follows_the_card_and_the_flag_overrides_it() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("add task T parent 1", &store, &worker()).await.unwrap();
    // a fresh card runs at medium
    let script = Scripted::new(vec![says("done", vec![])]);
    let runner = Worker::new("claude-opus-5-5").with_provider(std::sync::Arc::new(Harness(script.clone())));
    run("run 2", &store, &runner).await.unwrap();
    assert_eq!(script.seen.lock()[0].effort.as_deref(), Some("medium"));
    // `effort` sets the card, and the file carries it
    let (out, _) = run("effort 2 high", &store, &worker()).await.unwrap();
    assert!(out[0].contains("high"), "{out:?}");
    assert_eq!(store.read(2).await.unwrap().effort, ra_roadmap::model::Effort::High);
    let script = Scripted::new(vec![says("done", vec![])]);
    let runner = Worker::new("claude-opus-5-5").with_provider(std::sync::Arc::new(Harness(script.clone())));
    run("run 2", &store, &runner).await.unwrap();
    assert_eq!(script.seen.lock()[0].effort.as_deref(), Some("high"));
    // --effort overrides the card for one run
    let script = Scripted::new(vec![says("done", vec![])]);
    let runner = Worker::new("claude-opus-5-5")
        .with_provider(std::sync::Arc::new(Harness(script.clone())))
        .with_effort(Some(ra_roadmap::model::Effort::Low));
    run("run 2", &store, &runner).await.unwrap();
    assert_eq!(script.seen.lock()[0].effort.as_deref(), Some("low"));
    assert!(run("effort 2 extreme", &store, &worker()).await.is_err());
}

#[tokio::test]
async fn a_bare_model_stops_at_the_turn_cap() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    run("add task T parent 1", &store, &worker()).await.unwrap();
    let read = |n: &str| ra_providers::ToolCall { id: n.into(), name: "list_dir".into(), arguments: r#"{"path":"."}"#.into() };
    let provider = Scripted::new(vec![says("looking", vec![read("c1")]), says("still", vec![read("c2")]), says("never reached", vec![])]);
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(provider.clone()).with_turns(Some(2));
    let error = run("run 2", &store, &runner).await.unwrap_err();
    assert!(format!("{error:#}").contains("2 turns"), "{error:#}");
    assert_eq!(provider.seen.lock().len(), 2);
}

#[tokio::test]
async fn a_run_past_its_budget_stops_and_fails_the_node_with_a_note() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    // with budgets on, a c3 card has a $1.00 budget; off by default, nothing caps the run
    run("add task T parent 1", &store, &worker()).await.unwrap();
    let read = |n: &str| ra_providers::ToolCall { id: n.into(), name: "list_dir".into(), arguments: r#"{"path":"."}"#.into() };
    let costly = |calls| {
        let mut answer = says("working", calls);
        answer.usage.cost_usd = Some(0.6);
        answer
    };
    let provider = Scripted::new(vec![costly(vec![read("c1")]), costly(vec![read("c2")]), says("never reached", vec![])]);
    let runner = Worker::new("z-ai/glm-5.3-flash").with_provider(provider.clone()).with_budgets(true);
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
