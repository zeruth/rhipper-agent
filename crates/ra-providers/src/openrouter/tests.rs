//! The tests of the adapter: the answer shapes a reasoning model returns, over a local server.

use super::*;
use crate::ModelProvider;
use std::io::{Read, Write};
use std::net::TcpListener;

/// The path a test answers on: it waits for one request and replies with `status` and `body`.
fn serve(status_line: &'static str, body: &'static str) -> String {
    serve_capturing(status_line, body).0
}

/// [`serve`], with the raw request the server read handed back on the channel.
fn serve_capturing(status_line: &'static str, body: &'static str) -> (String, std::sync::mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        if let Ok((mut socket, _)) = listener.accept() {
            let mut buffer = [0u8; 4096];
            let read = socket.read(&mut buffer).unwrap_or(0);
            let _ = sender.send(String::from_utf8_lossy(&buffer[..read]).into_owned());
            let response = format!(
                "{status_line}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes());
        }
    });
    (format!("http://127.0.0.1:{port}"), receiver)
}

/// A server that answers by path: `routes` pairs a path with the body it returns, every other
/// path gets a 404 with an OpenAI-shaped error. It serves until the test ends.
fn serve_routes(routes: Vec<(&'static str, &'static str)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut socket in listener.incoming().flatten() {
            let mut buffer = [0u8; 4096];
            let read = socket.read(&mut buffer).unwrap_or(0);
            let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
            let path = request.split_whitespace().nth(1).unwrap_or("");
            let (status_line, body) = routes
                .iter()
                .find(|(p, _)| *p == path)
                .map_or(("HTTP/1.1 404 Not Found", r#"{"error":{"message":"model 'local/x' not found"}}"#), |(_, body)| {
                    ("HTTP/1.1 200 OK", *body)
                });
            let response = format!(
                "{status_line}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes());
        }
    });
    format!("http://127.0.0.1:{port}/v1")
}

/// The model list a llama.cpp or Ollama server answers on `GET /v1/models`.
const TWO_MODELS: &str = r#"{"object":"list","data":[{"id":"qwen3-8b","object":"model"},{"id":"llama-3.2-3b","object":"model"}]}"#;

/// A URL on 127.0.0.1 with nothing listening: the port of a listener that was just dropped.
fn closed_port() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    format!("http://127.0.0.1:{port}/v1")
}

fn model() -> &'static str {
    "z-ai/glm-5.3-flash"
}

#[tokio::test]
async fn a_reasoning_answer_yields_the_reasoning() {
    let base = serve("HTTP/1.1 200 OK", r#"{"choices":[{"message":{"content":null,"reasoning":"the answer is 4","role":"assistant"}}]}"#);
    let provider = OpenRouter::at(&base, "test").unwrap();
    let answer = provider.complete(request()).await.unwrap();
    assert_eq!(answer.text, "the answer is 4");
}

#[tokio::test]
async fn a_plain_answer_yields_the_content() {
    let base = serve("HTTP/1.1 200 OK", r#"{"choices":[{"message":{"content":"pong","role":"assistant"}}]}"#);
    let provider = OpenRouter::at(&base, "test").unwrap();
    let answer = provider.complete(request()).await.unwrap();
    assert_eq!(answer.text, "pong");
}

#[tokio::test]
async fn an_empty_answer_names_its_body() {
    let base = serve("HTTP/1.1 200 OK", r#"{"choices":[{"message":{"content":null}}]}"#);
    let provider = OpenRouter::at(&base, "test").unwrap();
    let error = provider.complete(request()).await.unwrap_err();
    assert!(format!("{error}").contains("holds no text"), "{error}");
}

#[tokio::test]
async fn an_unparsable_body_names_its_start() {
    let base = serve("HTTP/1.1 200 OK", "<html>gateway</html>");
    let provider = OpenRouter::at(&base, "test").unwrap();
    let error = provider.complete(request()).await.unwrap_err();
    assert!(format!("{error}").contains("unparsable answer"), "{error}");
    assert!(format!("{error}").contains("gateway"), "{error}");
}

#[tokio::test]
async fn a_tool_call_answer_yields_the_calls() {
    let base = serve(
        "HTTP/1.1 200 OK",
        r#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"src/main.rs\"}"}}]},"role":"assistant"}]}"#,
    );
    let provider = OpenRouter::at(&base, "test").unwrap();
    let answer = provider.complete(request()).await.unwrap();
    assert_eq!(answer.tool_calls.len(), 1);
    let call = &answer.tool_calls[0];
    assert_eq!(call.id, "call_1");
    assert_eq!(call.name, "read_file");
    assert_eq!(call.arguments, r#"{"path":"src/main.rs"}"#);
}

#[tokio::test]
async fn a_tool_call_answer_without_arguments_defaults_to_an_empty_object() {
    let base = serve(
        "HTTP/1.1 200 OK",
        r#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"call_2","type":"function","function":{"name":"list_dir"}}]},"role":"assistant"}]}"#,
    );
    let provider = OpenRouter::at(&base, "test").unwrap();
    let answer = provider.complete(request()).await.unwrap();
    assert_eq!(answer.tool_calls[0].arguments, "{}");
}

#[tokio::test]
async fn a_tool_result_message_round_trips() {
    let base = serve("HTTP/1.1 200 OK", r#"{"choices":[{"message":{"content":"the file holds 3 lines","role":"assistant"}}]}"#);
    let provider = OpenRouter::at(&base, "test").unwrap();
    let mut request = request();
    request.messages = vec![
        Message::user("read src/main.rs"),
        Message::assistant_calls(
            "",
            vec![ToolCall { id: "call_1".into(), name: "read_file".into(), arguments: r#"{"path":"src/main.rs"}"#.into() }],
        ),
        Message::tool("call_1", "fn main() {}"),
    ];
    let answer = provider.complete(request).await.unwrap();
    assert_eq!(answer.text, "the file holds 3 lines");
}

#[tokio::test]
async fn a_tool_call_answer_with_no_text_is_not_a_refusal() {
    let base = serve(
        "HTTP/1.1 200 OK",
        r#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"read_file","arguments":"{}"}}]},"role":"assistant"}]}"#,
    );
    let provider = OpenRouter::at(&base, "test").unwrap();
    let answer = provider.complete(request()).await.unwrap();
    assert!(answer.text.is_empty());
    assert_eq!(answer.tool_calls.len(), 1);
}

#[tokio::test]
async fn a_local_server_answers_a_ping_without_a_key() {
    let (base, request_seen) = serve_capturing("HTTP/1.1 200 OK", r#"{"choices":[{"message":{"content":"pong","role":"assistant"}}]}"#);
    // a trailing slash on the root reaches the same endpoint
    let provider = OpenRouter::at(&format!("{base}/v1/"), "").unwrap();
    assert_eq!(provider.base_url(), format!("{base}/v1"));
    let mut request = request();
    request.model = "local/qwen3".to_owned();
    let answer = provider.complete(request).await.unwrap();
    assert_eq!(answer.text, "pong");
    let raw = request_seen.recv().unwrap();
    assert!(raw.starts_with("POST /v1/chat/completions "), "{raw}");
    assert!(!raw.to_ascii_lowercase().contains("authorization:"), "an empty key sends no header: {raw}");
}

#[tokio::test]
async fn a_key_goes_out_as_a_bearer_header() {
    let (base, request_seen) = serve_capturing("HTTP/1.1 200 OK", r#"{"choices":[{"message":{"content":"pong","role":"assistant"}}]}"#);
    OpenRouter::at(&base, "sk-or-test").unwrap().complete(request()).await.unwrap();
    assert!(request_seen.recv().unwrap().contains("authorization: Bearer sk-or-test"), "the key rides as a bearer token");
}

#[tokio::test]
async fn nothing_listening_is_unreachable_and_names_the_url() {
    let base = closed_port();
    let provider = OpenRouter::at(&base, "").unwrap();
    let error = provider.complete(request()).await.unwrap_err();
    assert!(matches!(error, ProviderError::Unavailable(_)), "{error}");
    assert!(format!("{error}").contains(&format!("{base}/chat/completions")), "{error}");
}

#[tokio::test]
async fn a_local_server_lists_its_models() {
    let base = serve_routes(vec![("/v1/models", TWO_MODELS)]);
    let models = OpenRouter::at(&base, "").unwrap().models().await.unwrap();
    assert_eq!(models, ["qwen3-8b", "llama-3.2-3b"]);
}

#[tokio::test]
async fn an_unknown_model_on_a_local_server_names_what_it_offers() {
    let base = serve_routes(vec![("/v1/models", TWO_MODELS)]);
    let provider = OpenRouter::at(&base, "").unwrap();
    let mut request = request();
    request.model = "local/x".to_owned();
    let error = provider.complete(request).await.unwrap_err();
    assert!(matches!(error, ProviderError::UnknownModel(_)), "{error}");
    let text = format!("{error}");
    assert!(text.contains("not found"), "{text}");
    assert!(text.contains(&format!("the server at {base} offers: qwen3-8b, llama-3.2-3b")), "{text}");
}

#[tokio::test]
async fn an_unknown_model_on_openrouter_lists_nothing() {
    // the public list runs to hundreds of ids; only a local server's is named
    let base = serve_routes(vec![("/v1/models", TWO_MODELS)]);
    let error = OpenRouter::at(&base, "sk-or-test").unwrap().complete(request()).await.unwrap_err();
    assert!(matches!(error, ProviderError::UnknownModel(_)), "{error}");
    assert!(!format!("{error}").contains("offers"), "{error}");
}

#[tokio::test]
async fn the_probe_goes_green_with_a_server_up_and_red_with_none() {
    let probe = LocalProbe::default();
    assert_eq!(probe.up(), None, "unprobed at first");
    probe.clone().probe(&serve_routes(vec![("/v1/models", TWO_MODELS)])).await;
    assert_eq!(probe.up(), Some(true));
    probe.clone().probe(&closed_port()).await;
    assert_eq!(probe.up(), Some(false));
}

#[test]
fn the_local_url_defaults_to_llama_cpp_and_reads_the_variable() {
    // the variable is read, not set, by this test: the default holds unless the machine sets it
    let expected = std::env::var(LOCAL_URL_VAR).ok().filter(|u| !u.trim().is_empty()).unwrap_or_else(|| DEFAULT_LOCAL_URL.to_owned());
    assert_eq!(local_url(), expected.trim());
    assert_eq!(DEFAULT_LOCAL_URL, "http://127.0.0.1:8080/v1");
}

#[test]
fn a_plain_message_sends_no_tool_calls_key() {
    let wire = serde_json::to_value(to_wire(&Message::user("hello"))).unwrap();
    assert_eq!(wire, serde_json::json!({"role": "user", "content": "hello"}));
}

#[test]
fn an_assistant_message_with_calls_and_no_text_sends_no_content() {
    let calls = vec![ToolCall { id: "call_1".into(), name: "read_file".into(), arguments: "{}".into() }];
    let wire = serde_json::to_value(to_wire(&Message::assistant_calls("", calls))).unwrap();
    assert!(wire.get("content").is_none(), "{wire}");
    assert_eq!(wire["tool_calls"][0]["function"]["name"], "read_file");
    // a tool result keeps its content even when empty
    let wire = serde_json::to_value(to_wire(&Message::tool("call_1", ""))).unwrap();
    assert_eq!(wire, serde_json::json!({"role": "tool", "content": "", "tool_call_id": "call_1"}));
}

fn request() -> CompletionRequest {
    CompletionRequest {
        model: model().to_owned(),
        messages: vec![Message::user("hello")],
        temperature: Some(0.0),
        max_tokens: Some(16),
        tools: Vec::new(),
        workspace: None,
        budget_usd: None,
        resume: None,
        max_turns: None,
        effort: None,
    }
}
