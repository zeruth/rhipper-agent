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

fn request() -> CompletionRequest {
    CompletionRequest { model: model().to_owned(), messages: vec![Message::user("hello")], temperature: Some(0.0), max_tokens: Some(16) }
}
