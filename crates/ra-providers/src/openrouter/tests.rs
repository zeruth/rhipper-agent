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

/// An answer that carries its tokens and no cost, as a local server sends every one and the
/// public service some.
const UNCOSTED: &str =
    r#"{"choices":[{"message":{"content":"pong","role":"assistant"}}],"usage":{"prompt_tokens":1000,"completion_tokens":500}}"#;

/// The public model list with its prices as decimal strings per token, and a model without any.
const PRICED_MODELS: &str = r#"{"data":[{"id":"z-ai/glm-5.3-flash","pricing":{"prompt":"0.000002","completion":"0.000004"}},{"id":"free/one","pricing":{"prompt":"0","completion":"0"}},{"id":"local-file"}]}"#;

fn prices_fixture(read_at: u64) -> Prices {
    let mut models = BTreeMap::new();
    models.insert(model().to_owned(), Price { prompt: 0.000_001, completion: 0.000_002, ..Price::default() });
    Prices { read_at, models }
}

#[tokio::test]
async fn an_answer_without_a_cost_is_priced_from_a_fresh_prices_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("prices.json");
    prices_fixture(now()).save(&path).unwrap();
    // the server answers the completion only: a fresh file is not refetched
    let base = serve("HTTP/1.1 200 OK", UNCOSTED);
    let provider = OpenRouter::at(&base, "test").unwrap().with_prices_at(&path);
    let answer = provider.complete(request()).await.unwrap();
    let cost = answer.usage.cost_usd.expect("priced from the file");
    assert!((cost - 0.002).abs() < 1e-12, "{cost}");
    assert!(answer.usage.cost_computed, "the usage says the cost was computed");
    assert_eq!(answer.usage.prompt_tokens, 1000);
}

#[tokio::test]
async fn a_stale_prices_file_is_read_again_from_the_endpoint_and_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("prices.json");
    prices_fixture(now() - PRICES_FOR.as_secs() - 1).save(&path).unwrap();
    let base = serve_routes(vec![("/v1/models", PRICED_MODELS), ("/v1/chat/completions", UNCOSTED)]);
    let provider = OpenRouter::at(&base, "test").unwrap().with_prices_at(&path);
    let answer = provider.complete(request()).await.unwrap();
    // the endpoint's prices, not the file's: 1000 × 0.000002 + 500 × 0.000004
    let cost = answer.usage.cost_usd.expect("priced from the endpoint");
    assert!((cost - 0.004).abs() < 1e-12, "{cost}");
    assert!(answer.usage.cost_computed);
    let written = Prices::load(&path).expect("the file was rewritten");
    assert!(written.fresh_at(now()));
    assert_eq!(written.models.keys().collect::<Vec<_>>(), ["free/one", model()], "a model without a price is left out");
    assert_eq!(written.models["free/one"], Price::default());
}

#[tokio::test]
async fn a_reported_cost_stays_as_reported_and_a_local_answer_is_free() {
    let base = serve(
        "HTTP/1.1 200 OK",
        r#"{"choices":[{"message":{"content":"pong","role":"assistant"}}],"usage":{"prompt_tokens":10,"completion_tokens":2,"cost":0.5}}"#,
    );
    let answer = OpenRouter::at(&base, "test").unwrap().complete(request()).await.unwrap();
    assert_eq!((answer.usage.cost_usd, answer.usage.cost_computed), (Some(0.5), false));
    // a local server prices nothing, and nothing is what it costs
    let base = serve("HTTP/1.1 200 OK", UNCOSTED);
    let answer = OpenRouter::at(&base, "").unwrap().complete(request()).await.unwrap();
    assert_eq!((answer.usage.cost_usd, answer.usage.cost_computed), (Some(0.0), true));
}

#[test]
fn a_model_the_prices_do_not_list_keeps_no_cost() {
    let prices = prices_fixture(now());
    let usage = Usage { prompt_tokens: 1, completion_tokens: 1, ..Usage::default() };
    assert_eq!(prices.cost("other/model", &usage), None);
    assert!(prices.cost(model(), &usage).is_some());
    assert!(!prices_fixture(0).fresh_at(now()), "a list from the epoch is stale");
}

#[test]
fn cached_tokens_are_priced_at_the_cache_rate_and_a_claude_model_from_the_one_table() {
    // a listed cache rate prices the cached share; a model without one prices it as input
    let mut prices = prices_fixture(now());
    prices.models.insert("cached/model".to_owned(), Price { prompt: 1e-6, completion: 0.0, cache_read: Some(1e-7), cache_write: None });
    let usage = Usage { prompt_tokens: 1000, cached_tokens: 900, ..Usage::default() };
    assert!((prices.cost("cached/model", &usage).unwrap() - (100e-6 + 900e-7)).abs() < 1e-12);
    assert!((prices.cost(model(), &usage).unwrap() - 1000e-6).abs() < 1e-12);
    // a Claude model the list leaves out takes the Anthropic list, alias or id
    let opus = prices.price_of("anthropic/claude-opus-4.5").expect("the Anthropic list");
    assert!((opus.prompt - 5e-6).abs() < 1e-15 && (opus.cache_read.unwrap() - 5e-7).abs() < 1e-15);
    assert_eq!(anthropic_price("opus"), Some(opus));
    assert!((anthropic_price("claude-opus-4-1-20250805").unwrap().prompt - 15e-6).abs() < 1e-15);
    assert!((anthropic_price("claude-sonnet-4-6").unwrap().completion - 15e-6).abs() < 1e-15);
    assert_eq!(anthropic_price("z-ai/glm-5.3-flash"), None);
}

#[test]
fn the_endpoint_cache_rates_are_read_with_the_prices() {
    let models: WireModels = serde_json::from_str(
        r#"{"data":[{"id":"anthropic/claude-sonnet-4.6","pricing":{"prompt":"0.000003","completion":"0.000015","input_cache_read":"0.0000003","input_cache_write":"0.00000375"}}]}"#,
    )
    .unwrap();
    let prices = Prices::from_wire(models.data, 1);
    let price = prices.models["anthropic/claude-sonnet-4.6"];
    assert_eq!((price.cache_read, price.cache_write), (Some(0.000_000_3), Some(0.000_003_75)));
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
        max_context: None,
        effort: None,
        read_only: false,
    }
}
