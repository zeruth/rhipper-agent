//! The tests of the adapter: the answer shapes a reasoning model returns, over a local server.

use super::*;
use crate::ModelProvider;
use std::io::{Read, Write};
use std::net::TcpListener;

/// The path a test answers on: it waits for one request and replies with `status` and `body`.
fn serve(status_line: &'static str, body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        if let Ok((mut socket, _)) = listener.accept() {
            let mut buffer = [0u8; 4096];
            let _ = socket.read(&mut buffer);
            let response = format!(
                "{status_line}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes());
        }
    });
    format!("http://127.0.0.1:{port}")
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
    }
}
