//! The tests of the adapter: the arguments it builds and the result JSON it reads. No test runs
//! the real binary.

use super::*;

fn request(workspace: Option<PathBuf>) -> CompletionRequest {
    CompletionRequest {
        model: "claude-opus-5-5".to_owned(),
        messages: vec![Message::system("You answer in one word."), Message::user("Reply with the single word: pong.")],
        temperature: None,
        max_tokens: None,
        tools: Vec::new(),
        workspace,
        budget_usd: None,
    }
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
    assert!(args.windows(2).any(|w| w == ["--system-prompt", "You answer in one word."]), "{args:?}");
    assert!(args.windows(2).any(|w| w == ["--tools", ""]), "{args:?}");
    assert!(args.windows(2).any(|w| w == ["--max-turns", "1"]), "{args:?}");
    assert!(!args.contains(&"--permission-mode".to_owned()), "{args:?}");
}

#[test]
fn a_worker_request_gets_the_file_tools_and_the_allowed_shell() {
    let args = ClaudeCode::new().with_max_turns(7).args(&request(Some(PathBuf::from("."))));
    assert!(!args.contains(&"--bare".to_owned()), "{args:?}");
    assert!(args.contains(&"--no-session-persistence".to_owned()), "{args:?}");
    assert!(args.windows(2).any(|w| w == ["--permission-mode", "acceptEdits"]), "{args:?}");
    assert!(args.windows(2).any(|w| w == ["--max-turns", "7"]), "{args:?}");
    let at = args.iter().position(|a| a == "--allowedTools").unwrap();
    assert_eq!(args[at + 1], "Bash(cargo *)");
    assert!(!args.contains(&"--tools".to_owned()), "{args:?}");
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
    let json = r#"{"type":"result","subtype":"error_max_turns","is_error":true,"result":""}"#;
    assert!(matches!(parse_result(json).unwrap_err(), ProviderError::Other(text) if text.contains("error_max_turns")));
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
fn garbage_names_its_start() {
    let error = parse_result("Not a JSON line").unwrap_err();
    assert!(format!("{error}").contains("Not a JSON"), "{error}");
}

#[tokio::test]
async fn a_missing_binary_is_unavailable() {
    let provider = ClaudeCode::new().with_binary("no-such-claude-binary-anywhere");
    let error = provider.complete(request(None)).await.unwrap_err();
    assert!(matches!(&error, ProviderError::Unavailable(text) if text.contains("not installed")), "{error}");
}
