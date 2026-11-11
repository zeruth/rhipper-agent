//! The tests of the adapter: the arguments it builds and the result JSON it reads. No test runs
//! the real binary.

use super::*;
use crate::turns_for;

fn request(workspace: Option<PathBuf>) -> CompletionRequest {
    CompletionRequest {
        model: "claude-opus-5-5".to_owned(),
        messages: vec![Message::system("You answer in one word."), Message::user("Reply with the single word: pong.")],
        temperature: None,
        max_tokens: None,
        tools: Vec::new(),
        workspace,
        budget_usd: None,
        resume: None,
        max_turns: None,
        max_context: None,
        effort: None,
    }
}

#[test]
fn the_effort_follows_the_card_and_stays_medium_without_one() {
    let worker = request(Some(PathBuf::from(".")));
    let args = ClaudeCode::new().args(&worker);
    assert!(args.windows(2).any(|w| w == ["--effort", "medium"]), "{args:?}");
    let mut hard = request(Some(PathBuf::from(".")));
    hard.effort = Some("high".to_owned());
    let args = ClaudeCode::new().args(&hard);
    assert!(args.windows(2).any(|w| w == ["--effort", "high"]), "{args:?}");
    assert_eq!(args.iter().filter(|a| *a == "--effort").count(), 1, "{args:?}");
    // the card's level wins over the provider's; a blank one does not
    let args = ClaudeCode::new().with_effort("low").args(&hard);
    assert!(args.windows(2).any(|w| w == ["--effort", "high"]), "{args:?}");
    hard.effort = Some(" ".to_owned());
    let args = ClaudeCode::new().with_effort("low").args(&hard);
    assert!(args.windows(2).any(|w| w == ["--effort", "low"]), "{args:?}");
}

#[test]
fn the_turn_cap_follows_the_complexity_unless_the_request_names_one() {
    assert_eq!([1, 2, 3, 4, 5].map(turns_for), [15, 25, 40, 60, 90]);
    for complexity in 1..=5 {
        let mut worker = request(Some(PathBuf::from(".")));
        worker.max_turns = Some(turns_for(complexity));
        let args = ClaudeCode::new().args(&worker);
        let cap = turns_for(complexity).to_string();
        assert!(args.windows(2).any(|w| w == ["--max-turns", cap.as_str()]), "c{complexity}: {args:?}");
    }
    // `rc run --turns 12` overrides the complexity; a request without a cap takes the provider's
    let mut worker = request(Some(PathBuf::from(".")));
    worker.max_turns = Some(12);
    let args = ClaudeCode::new().with_max_turns(60).args(&worker);
    assert!(args.windows(2).any(|w| w == ["--max-turns", "12"]), "{args:?}");
    // the cap stays on without a budget: the turns are the cost on a subscription
    assert!(!args.contains(&"--max-budget-usd".to_owned()), "{args:?}");
    let json = r#"{"type":"result","subtype":"success","is_error":false,"num_turns":12,"result":"done","session_id":"s"}"#;
    assert_eq!(parse_result(json).unwrap().turns, 12);
}

#[test]
fn a_budget_goes_on_the_command_line() {
    let mut worker = request(Some(PathBuf::from(".")));
    worker.budget_usd = Some(0.5);
    let args = ClaudeCode::new().args(&worker);
    assert!(args.windows(2).any(|w| w == ["--max-budget-usd", "0.50"]), "{args:?}");
    // no budget, no flag
    assert!(!ClaudeCode::new().args(&request(None)).contains(&"--max-budget-usd".to_owned()));
}

#[test]
fn a_chat_request_runs_with_no_tools_and_one_turn() {
    let args = ClaudeCode::new().args(&request(None));
    assert_eq!(&args[..5], ["-p", "--output-format", "stream-json", "--verbose", "--no-session-persistence"]);
    // --bare would drop the OAuth login the subscription route runs on
    assert!(!args.contains(&"--bare".to_owned()), "{args:?}");
    assert!(args.windows(2).any(|w| w == ["--model", "claude-opus-5-5"]), "{args:?}");
    assert!(args.windows(2).any(|w| w == ["--effort", "medium"]), "{args:?}");
    assert!(ClaudeCode::new().with_effort("high").args(&request(None)).windows(2).any(|w| w == ["--effort", "high"]));
    assert!(args.windows(2).any(|w| w == ["--system-prompt", "You answer in one word."]), "{args:?}");
    assert!(args.windows(2).any(|w| w == ["--tools", ""]), "{args:?}");
    assert!(args.windows(2).any(|w| w == ["--max-turns", "1"]), "{args:?}");
    assert!(!args.contains(&"--permission-mode".to_owned()), "{args:?}");
}

#[test]
fn a_worker_request_gets_the_file_tools_and_the_allowed_shell() {
    let args = ClaudeCode::new().with_max_turns(7).args(&request(Some(PathBuf::from("."))));
    assert!(!args.contains(&"--bare".to_owned()), "{args:?}");
    // a worker keeps its session, for the next run of the task to resume
    assert!(!args.contains(&"--no-session-persistence".to_owned()), "{args:?}");
    assert!(!args.contains(&"--resume".to_owned()), "{args:?}");
    assert!(args.windows(2).any(|w| w == ["--permission-mode", "acceptEdits"]), "{args:?}");
    assert!(args.windows(2).any(|w| w == ["--max-turns", "7"]), "{args:?}");
    let at = args.iter().position(|a| a == "--allowedTools").unwrap();
    assert_eq!(args[at + 1], "Bash(cargo *)");
    // a worker gets the built-in list, never the empty list of a chat, and the project's
    // settings only: the user's hooks are for their own sessions
    assert!(!args.windows(2).any(|w| w == ["--tools", ""]), "{args:?}");
    assert!(args.windows(2).any(|w| w == ["--setting-sources", "project,local"]), "{args:?}");
    // no MCP servers: the project's own would be the agent itself
    assert!(args.contains(&"--strict-mcp-config".to_owned()), "{args:?}");
    assert!(!ClaudeCode::new().args(&request(None)).contains(&"--strict-mcp-config".to_owned()));
    assert!(!ClaudeCode::new().args(&request(None)).contains(&"--setting-sources".to_owned()));
}

#[test]
fn a_known_session_is_resumed() {
    let mut worker = request(Some(PathBuf::from(".")));
    worker.resume = Some("3f2a9c1e".to_owned());
    let args = ClaudeCode::new().args(&worker);
    assert!(args.windows(2).any(|w| w == ["--resume", "3f2a9c1e"]), "{args:?}");
}

#[test]
fn the_prompt_is_the_lone_user_message_or_a_transcript() {
    assert_eq!(prompt_of(&request(None).messages), "Reply with the single word: pong.");
    let messages = vec![Message::user("hi"), Message::assistant_calls("hello", Vec::new()), Message::user("bye")];
    assert_eq!(prompt_of(&messages), "user: hi\n\nassistant: hello\n\nuser: bye");
}

#[test]
fn a_result_yields_the_text_the_usage_and_the_cost() {
    let json = r#"{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"pong","total_cost_usd":0.041228,
        "usage":{"input_tokens":10,"cache_creation_input_tokens":20479,"cache_read_input_tokens":5,"output_tokens":52},
        "permission_denials":[],"session_id":"x"}"#;
    let answer = parse_result(json).unwrap();
    assert_eq!(answer.text, "pong");
    assert_eq!(answer.session_id.as_deref(), Some("x"));
    assert_eq!(answer.usage.prompt_tokens, 10 + 20479 + 5);
    assert_eq!(answer.usage.completion_tokens, 52);
    assert_eq!(answer.usage.cached_tokens, 5);
    assert_eq!(answer.usage.cost_usd, Some(0.041228));
    assert!(answer.tool_calls.is_empty());
}

#[test]
fn an_error_result_maps_its_status() {
    let json = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"Not logged in","api_error_status":401}"#;
    assert!(matches!(parse_result(json).unwrap_err(), ProviderError::Auth(text) if text.contains("Not logged in")));
    let json = r#"{"type":"result","subtype":"error","is_error":true,"result":"no such model","api_error_status":404}"#;
    assert!(matches!(parse_result(json).unwrap_err(), ProviderError::UnknownModel(_)));
    let json = r#"{"type":"result","subtype":"error","is_error":true,"result":"overloaded","api_error_status":529}"#;
    assert!(matches!(parse_result(json).unwrap_err(), ProviderError::Unavailable(_)));
}

#[test]
fn a_run_out_of_turns_comes_back_with_its_usage_and_session() {
    let json = r#"{"type":"result","subtype":"error_max_turns","is_error":true,"result":"","num_turns":40,"total_cost_usd":1.25,
        "usage":{"input_tokens":10,"cache_creation_input_tokens":90,"cache_read_input_tokens":900,"output_tokens":52},"session_id":"s-40"}"#;
    let error = parse_result(json).unwrap_err();
    let ProviderError::TurnsOut(answer) = error else { panic!("{error}") };
    assert_eq!(answer.turns, 40);
    assert_eq!(answer.usage.prompt_tokens, 1000);
    assert_eq!(answer.usage.completion_tokens, 52);
    assert_eq!(answer.usage.cost_usd, Some(1.25));
    assert_eq!(answer.session_id.as_deref(), Some("s-40"));
}

#[test]
fn a_logged_out_harness_is_an_auth_error_even_without_a_status() {
    let json = r#"{"type":"result","subtype":"success","is_error":true,"api_error_status":null,"terminal_reason":"api_error","result":"Not logged in · Please run /login"}"#;
    assert!(matches!(parse_result(json).unwrap_err(), ProviderError::Auth(text) if text.contains("Not logged in")));
}

#[test]
fn the_stream_shows_tool_calls_and_text_but_not_thinking() {
    let root = Path::new(r"C:\work\repo");
    let line = r#"{"type":"assistant","message":{"content":[
        {"type":"thinking","thinking":"secret"},
        {"type":"text","text":"\nI will check the store first.\nThen the tests."},
        {"type":"tool_use","name":"Read","input":{"file_path":"C:\\work\\repo\\crates\\store.rs"}},
        {"type":"tool_use","name":"Bash","input":{"command":"cargo test -p ra-roadmap"}},
        {"type":"tool_use","name":"Grep","input":{"pattern":"fn insert"}},
        {"type":"tool_use","name":"TodoWrite","input":{}}]}}"#
        .replace('\n', " ");
    assert_eq!(
        progress_of(&line, Some(root)),
        ["… I will check the store first.", "Read crates/store.rs", "$ cargo test -p ra-roadmap", "Grep fn insert", "TodoWrite"]
    );
    // the other events show nothing
    assert!(progress_of(r#"{"type":"user","message":{"content":[{"type":"tool_result"}]}}"#, Some(root)).is_empty());
    assert!(progress_of(r#"{"type":"result","result":"done"}"#, Some(root)).is_empty());
    assert!(progress_of("not json", Some(root)).is_empty());
}

#[test]
fn the_stream_tallies_the_tokens_of_each_turn() {
    // three turns: the second comes as two events of one message, and the last one read stands
    let stream = [
        r#"{"type":"system","subtype":"init","session_id":"s"}"#,
        r#"{"type":"assistant","message":{"id":"m1","usage":{"input_tokens":3,"cache_read_input_tokens":1000,"cache_creation_input_tokens":200,"output_tokens":40},"content":[{"type":"text","text":"looking"}]}}"#,
        r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"ok"}]}}"#,
        r#"{"type":"assistant","message":{"id":"m2","usage":{"input_tokens":5,"cache_read_input_tokens":1200,"output_tokens":10},"content":[{"type":"text","text":"editing"}]}}"#,
        r#"{"type":"assistant","message":{"id":"m2","usage":{"input_tokens":5,"cache_read_input_tokens":1200,"output_tokens":90},"content":[{"type":"tool_use","name":"Edit","input":{}}]}}"#,
        r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"ok"}]}}"#,
        r#"{"type":"assistant","message":{"id":"m3","usage":{"input_tokens":7,"cache_read_input_tokens":1500,"output_tokens":25},"content":[{"type":"text","text":"done"}]}}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"num_turns":3,"result":"done"}"#,
        "not json",
    ];
    let mut tally = TurnTally::default();
    for line in stream {
        tally.note(line);
    }
    assert_eq!(
        tally.into_turns(),
        [
            TurnUsage { input_tokens: 1203, output_tokens: 40 },
            TurnUsage { input_tokens: 1205, output_tokens: 90 },
            TurnUsage { input_tokens: 1507, output_tokens: 25 },
        ]
    );
}

#[test]
fn a_turn_past_the_context_cap_stops_the_run_once_its_tool_results_are_in() {
    // the second turn reads 130k: the run goes on through its tool call and stops at the result
    // of that call, before a third turn reads it all again
    let stream = [
        r#"{"type":"system","subtype":"init","session_id":"s"}"#,
        r#"{"type":"assistant","message":{"id":"m1","usage":{"input_tokens":3,"cache_read_input_tokens":90000,"output_tokens":40},"content":[{"type":"text","text":"looking"}]}}"#,
        r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"ok"}]}}"#,
        r#"{"type":"assistant","message":{"id":"m2","usage":{"input_tokens":5,"cache_read_input_tokens":120000,"cache_creation_input_tokens":10000,"output_tokens":10},"content":[{"type":"text","text":"editing"}]}}"#,
        r#"{"type":"assistant","message":{"id":"m2","usage":{"input_tokens":5,"cache_read_input_tokens":120000,"cache_creation_input_tokens":10000,"output_tokens":90},"content":[{"type":"tool_use","name":"Edit","input":{}}]}}"#,
        r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"ok"}]}}"#,
        r#"{"type":"assistant","message":{"id":"m3","usage":{"input_tokens":7,"cache_read_input_tokens":131000,"output_tokens":25},"content":[{"type":"text","text":"done"}]}}"#,
    ];
    let mut tally = TurnTally::default();
    let mut watch = ContextWatch::new(Some(120_000));
    let mut stopped = None;
    let mut read = 0;
    for line in stream {
        tally.note(line);
        read += 1;
        if let Some(size) = watch.note(line, &tally) {
            stopped = Some(size);
            break;
        }
    }
    assert_eq!((stopped, read), (Some(130_005), 6), "the run stops at the tool result of the crossing turn");
    let answer = tally.completion();
    assert_eq!(answer.turns, 2);
    assert_eq!(answer.usage.prompt_tokens, 90_003 + 130_005);
    assert_eq!(answer.usage.completion_tokens, 130);
    assert_eq!(answer.usage.cached_tokens, 210_000);
    assert_eq!(answer.usage.cost_usd, None);
    assert_eq!(answer.text, "editing");
    assert_eq!(answer.turn_usage.len(), 2);
    // without a cap nothing stops, and under it neither
    let mut tally = TurnTally::default();
    let mut none = ContextWatch::new(None);
    let mut wide = ContextWatch::new(Some(200_000));
    for line in stream {
        tally.note(line);
        assert_eq!(none.note(line, &tally), None);
        assert_eq!(wide.note(line, &tally), None);
    }
}

#[test]
fn the_gotchas_route_the_hooks_and_count_what_the_stream_shows() {
    let catalogue = Catalogue::seed();
    let provider = ClaudeCode::new().with_gotchas(r"C:\tools\rc.exe", catalogue.clone());
    let args = provider.args(&request(Some(PathBuf::from("."))));
    let at = args.iter().position(|a| a == "--settings").expect("--settings");
    let settings: serde_json::Value = serde_json::from_str(&args[at + 1]).unwrap();
    assert_eq!(settings["hooks"]["PreToolUse"][0]["matcher"], "Bash|PowerShell");
    assert_eq!(settings["hooks"]["PostToolUse"][0]["matcher"], "Bash|PowerShell|Edit|Write|MultiEdit|NotebookEdit|Read");
    assert_eq!(settings["hooks"]["PreToolUse"][0]["hooks"][0]["command"], "\"C:\\tools\\rc.exe\" hook pre");
    assert_eq!(settings["hooks"]["PostToolUse"][0]["hooks"][0]["command"], "\"C:\\tools\\rc.exe\" hook post");
    // a chat completion carries no hooks
    assert!(!ClaudeCode::new().with_gotchas("rc", Catalogue::seed()).args(&request(None)).contains(&"--settings".to_owned()));
    assert!(ClaudeCode::new().new_allowed().iter().any(|r| r == "Bash(ls*)"));
    assert!(ClaudeCode::new().new_allowed().iter().any(|r| r == "PowerShell(cargo *)"));
    // both shells, the file tools, no subagents
    assert!(
        args.windows(2).any(|w| w[0] == "--tools"
            && w[1].contains("Bash")
            && w[1].contains("PowerShell")
            && w[1].contains("Edit")
            && !w[1].split(',').any(|t| t == "Task")),
        "{args:?}"
    );
    assert!(!args.contains(&"--disallowedTools".to_owned()), "{args:?}");

    let quiet = Conditions::default();
    let mut tools = std::collections::HashMap::new();
    let denied =
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cd crates && ls"}}]}}"#;
    let hits = gotchas_of_event(&catalogue, denied, quiet, &mut tools);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "shell-outside-allowlist");
    assert_eq!(hits[0].excerpt, "cd crates && ls");
    // a read-only command is allowed now, and no hit
    let listing =
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t2","name":"Bash","input":{"command":"ls -la crates"}}]}}"#;
    assert!(gotchas_of_event(&catalogue, listing, quiet, &mut tools).is_empty());
    // a file the worker reads may quote any error; only a shell's output counts
    let reading =
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t3","name":"Read","input":{"file_path":"rhipper.md"}}]}}"#;
    assert!(gotchas_of_event(&catalogue, reading, quiet, &mut tools).is_empty());
    let quoted = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t3","content":"notes: Not logged in · Please run /login"}]}}"#;
    assert!(gotchas_of_event(&catalogue, quoted, quiet, &mut tools).is_empty());
    let fine = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"cargo test -p ra-roadmap 2>&1 | tail -n 40"}}]}}"#;
    assert!(gotchas_of_event(&catalogue, fine, quiet, &mut tools).is_empty());
    // a cargo test without a tail is rewritten, and the rewrite counts
    let untailed =
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"cargo test -p ra-roadmap"}}]}}"#;
    assert_eq!(gotchas_of_event(&catalogue, untailed, quiet, &mut tools)[0].id, "long-output");
    // the reads of an event, relative to the workspace; a Bash call or a Grep is not one
    let root = Path::new(r"C:\work\repo");
    let two = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"C:\\work\\repo\\crates\\c.rs"}},{"type":"tool_use","name":"Grep","input":{"pattern":"fn"}},{"type":"tool_use","name":"Read","input":{"file_path":"D:\\elsewhere\\x.rs"}}]}}"#;
    assert_eq!(reads_of(two, Some(root)), ["crates/c.rs", "D:/elsewhere/x.rs"]);
    assert!(reads_of(fine, Some(root)).is_empty());
    let locked = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t2","content":"error: failed to remove file `rc.exe`\nAccess is denied. (os error 5)"}]}}"#;
    assert_eq!(gotchas_of_event(&catalogue, locked, quiet, &mut tools)[0].id, "binary-locked-result");
    let claim = r#"{"type":"result","result":"Done, but I couldn't run any tests.","permission_denials":[{"tool_name":"Bash","tool_input":{"command":"make all"}}]}"#;
    let ids: Vec<String> = gotchas_of_event(&catalogue, claim, quiet, &mut tools).into_iter().map(|h| h.id).collect();
    assert_eq!(ids, ["untested-claim", "unmatched-denied"]);
    // the worker's own words about an edit gone wrong, and an edit tool's miss, each count
    let words = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Whoops, that edit landed in the wrong place."},{"type":"tool_use","id":"t4","name":"Edit","input":{"file_path":"x.rs"}}]}}"#;
    let ids: Vec<String> = gotchas_of_event(&catalogue, words, quiet, &mut tools).into_iter().map(|h| h.id).collect();
    assert_eq!(ids, ["misplaced-edit"]);
    let missed = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t4","content":"Error: String to replace not found in file."}]}}"#;
    let ids: Vec<String> = gotchas_of_event(&catalogue, missed, quiet, &mut tools).into_iter().map(|h| h.id).collect();
    assert_eq!(ids, ["edit-missed"]);
    // a PowerShell denial is judged like a Bash one: a command the catalogue passes is unmatched
    let refused = r#"{"type":"result","result":"done","permission_denials":[{"tool_name":"PowerShell","tool_input":{"command":"cargo clippy -p rc-gui"}}]}"#;
    let ids: Vec<String> = gotchas_of_event(&catalogue, refused, quiet, &mut tools).into_iter().map(|h| h.id).collect();
    assert_eq!(ids, ["unmatched-denied"]);
}

#[test]
fn garbage_names_its_start() {
    let error = parse_result("Not a JSON line").unwrap_err();
    assert!(format!("{error}").contains("Not a JSON"), "{error}");
}

#[test]
fn a_run_s_stream_is_kept_line_by_line_and_the_next_run_replaces_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut stream = Stream::open(Some(dir.path()));
    stream.note(r#"{"type":"system","subtype":"init"}"#);
    stream.note("");
    stream.note(r#"{"type":"result","result":"done"}"#);
    drop(stream);
    let text = std::fs::read_to_string(dir.path().join(STREAM_FILE)).unwrap();
    assert_eq!(text.lines().count(), 2, "{text}");
    assert!(text.ends_with("\"done\"}\n"), "{text}");
    // the next run starts the file over; a chat completion keeps none
    let mut stream = Stream::open(Some(dir.path()));
    stream.note(r#"{"type":"result","result":"again"}"#);
    drop(stream);
    assert_eq!(std::fs::read_to_string(dir.path().join(STREAM_FILE)).unwrap().lines().count(), 1);
    assert!(Stream::open(None).0.is_none());
}

#[tokio::test]
async fn a_missing_binary_is_unavailable() {
    let provider = ClaudeCode::new().with_binary("no-such-claude-binary-anywhere");
    let error = provider.complete(request(None)).await.unwrap_err();
    assert!(matches!(&error, ProviderError::Unavailable(text) if text.contains("not installed")), "{error}");
}
