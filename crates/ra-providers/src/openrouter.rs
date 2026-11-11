//! The OpenRouter adapter: one HTTP POST to `chat/completions` per completion. It maps the HTTP
//! statuses to the provider errors so a caller can retry the transient ones.
//!
//! The same adapter serves a local OpenAI-compatible server such as llama.cpp: [`OpenRouter::local`]
//! points it at the URL from `RHIPPER_LOCAL_URL` with no key. [`LocalProbe`] asks that server for
//! its model list in the background, so a status dot can say whether it is up without a model call.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use reqwest::StatusCode;
use serde::Deserialize;

use crate::{Completion, CompletionFuture, CompletionRequest, Message, ModelProvider, ProviderError, ToolCall, ToolSpec, Usage};

/// The API root of the public service.
const BASE_URL: &str = "https://openrouter.ai/api/v1";
/// The environment variable that names the local server's API root.
pub const LOCAL_URL_VAR: &str = "RHIPPER_LOCAL_URL";
/// The API root of a local server when the variable is unset: llama.cpp's default port.
pub const DEFAULT_LOCAL_URL: &str = "http://127.0.0.1:8080/v1";
/// What the requests say they are.
const USER_AGENT: &str = concat!("rhipper-agent/", env!("CARGO_PKG_VERSION"));
/// How long a probe of the local server waits: it answers from memory or not at all.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
/// How often the status dots of the console and the window probe the local server.
pub const PROBE_EVERY: Duration = Duration::from_secs(60);

/// The OpenRouter provider.
pub struct OpenRouter {
    client: reqwest::Client,
    /// The bearer key; empty sends no `Authorization` header, as a local server needs none.
    key: String,
    /// The API root; the tests point it at a local server.
    base_url: String,
}

impl OpenRouter {
    /// Builds the provider from an OpenRouter API key.
    pub fn new(key: impl Into<String>) -> anyhow::Result<Self> {
        Self::at(BASE_URL, key)
    }

    /// Builds the provider against the local server at [`local_url`], with no key.
    pub fn local() -> anyhow::Result<Self> {
        Self::at(&local_url(), "")
    }

    /// Builds the provider against another base URL, for tests and local servers. A trailing slash
    /// is dropped, so `http://host/v1/` and `http://host/v1` reach the same endpoint.
    pub fn at(base_url: &str, key: impl Into<String>) -> anyhow::Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder().user_agent(USER_AGENT).build()?,
            key: key.into(),
            base_url: base_url.trim_end_matches('/').to_owned(),
        })
    }

    /// The API root the provider posts to.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Whether the key is empty: the sign of a local server, which has no accounts.
    fn is_local(&self) -> bool {
        self.key.is_empty()
    }

    /// The ids the server offers, from `GET models`. A local server lists the files it loaded;
    /// `ping` names them when a model is unknown to it.
    pub async fn models(&self) -> Result<Vec<String>, ProviderError> {
        let url = format!("{}/models", self.base_url);
        let mut get = self.client.get(&url).timeout(PROBE_TIMEOUT);
        if !self.is_local() {
            get = get.bearer_auth(&self.key);
        }
        let response = get.send().await.map_err(|e| ProviderError::Unavailable(format!("cannot reach {url}: {e}")))?;
        let status = response.status();
        let body = response.text().await.map_err(|e| ProviderError::Unavailable(format!("{url} dropped the answer: {e}")))?;
        if status != StatusCode::OK {
            return Err(ProviderError::Other(error_text(&body, status)));
        }
        let parsed: WireModels = serde_json::from_str(&body)
            .map_err(|e| ProviderError::Other(format!("unparsable model list: {e}; body starts with: {}", snippet(&body))))?;
        Ok(parsed.data.into_iter().map(|m| m.id).collect())
    }

    /// The text of an unknown-model error. On a local server it goes on to name what the server
    /// offers, since the id was typed, not picked from a list.
    async fn unknown_model_text(&self, body: &str, status: StatusCode) -> String {
        let mut text = error_text(body, status);
        if self.is_local()
            && let Ok(models) = self.models().await
            && !models.is_empty()
        {
            text.push_str(&format!("; the server at {} offers: {}", self.base_url, models.join(", ")));
        }
        text
    }
}

/// The availability of the local server, as the status dots show it: probed in the background
/// with one `GET models` and no model call, shared between the probe and the frame that reads it.
#[derive(Clone, Default)]
pub struct LocalProbe {
    /// 0 unprobed, 1 down, 2 up.
    state: Arc<AtomicU8>,
}

impl LocalProbe {
    /// Whether the server answered the last probe; `None` before the first one answers.
    pub fn up(&self) -> Option<bool> {
        match self.state.load(Ordering::Relaxed) {
            0 => None,
            1 => Some(false),
            _ => Some(true),
        }
    }

    /// One probe of the server at `base_url`, to spawn on the runtime; it writes the outcome
    /// where [`up`](Self::up) reads it.
    pub async fn probe(self, base_url: &str) {
        let answered = match OpenRouter::at(base_url, "") {
            Ok(server) => server.models().await.is_ok(),
            Err(_) => false,
        };
        self.state.store(if answered { 2 } else { 1 }, Ordering::Relaxed);
    }

    /// [`probe`](Self::probe) at [`local_url`].
    pub async fn probe_local(self) {
        self.probe(&local_url()).await;
    }
}

#[derive(Deserialize)]
struct WireModels {
    #[serde(default)]
    data: Vec<WireModel>,
}

#[derive(Deserialize)]
struct WireModel {
    id: String,
}

/// The API root of the local server: `RHIPPER_LOCAL_URL` when it is set to something, else
/// [`DEFAULT_LOCAL_URL`].
pub fn local_url() -> String {
    std::env::var(LOCAL_URL_VAR).ok().map(|u| u.trim().to_owned()).filter(|u| !u.is_empty()).unwrap_or_else(|| DEFAULT_LOCAL_URL.to_owned())
}

/// The wire format of one chat message. Content is optional because tool messages may carry it
/// as null and reasoning models answer in `reasoning` instead.
#[derive(serde::Serialize, Deserialize)]
struct WireMessage<'a> {
    role: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<&'a str>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<WireToolCall<'a>>,
    /// The id of the call a tool message answers; `tool_call_id` on the wire.
    #[serde(rename = "tool_call_id", skip_serializing_if = "Option::is_none")]
    answering: Option<&'a str>,
}

#[derive(serde::Serialize, Deserialize)]
struct WireToolCall<'a> {
    id: &'a str,
    /// The call kind; OpenRouter expects `function`.
    #[serde(rename = "type")]
    kind: &'a str,
    function: WireFunction<'a>,
}

#[derive(serde::Serialize, Deserialize)]
struct WireFunction<'a> {
    name: &'a str,
    /// The arguments, a JSON object in text form.
    arguments: &'a str,
}

#[derive(serde::Serialize)]
struct WireTool<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    function: WireToolFunction<'a>,
}

#[derive(serde::Serialize)]
struct WireToolFunction<'a> {
    name: &'a str,
    description: &'a str,
    parameters: &'a serde_json::Value,
}

#[derive(serde::Serialize)]
struct WireRequest<'a> {
    model: &'a str,
    messages: Vec<WireMessage<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<WireTool<'a>>,
}

#[derive(Deserialize)]
struct WireResponse {
    choices: Vec<WireChoice>,
    #[serde(default)]
    usage: Option<WireUsage>,
}

#[derive(Deserialize)]
struct WireChoice {
    message: WireMessageOwned,
}

/// The message of an answer. A reasoning model answers in `reasoning` and leaves `content` null or
/// empty, so both fields are optional and the text takes whichever one holds something.
#[derive(Deserialize)]
struct WireMessageOwned {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    tool_calls: Vec<WireToolCallOwned>,
}

#[derive(Deserialize)]
struct WireToolCallOwned {
    id: String,
    function: WireFunctionOwned,
}

#[derive(Deserialize)]
struct WireFunctionOwned {
    name: String,
    #[serde(default)]
    arguments: Option<String>,
}

impl WireMessageOwned {
    /// The text of the answer: the content, else the reasoning.
    fn text(&self) -> String {
        self.content.clone().filter(|c| !c.trim().is_empty()).or_else(|| self.reasoning.clone()).unwrap_or_default()
    }

    /// The tool calls of the answer, with the arguments defaulting to an empty object.
    fn tool_calls(&self) -> Vec<ToolCall> {
        self.tool_calls
            .iter()
            .map(|c| ToolCall {
                id: c.id.clone(),
                name: c.function.name.clone(),
                arguments: c.function.arguments.clone().unwrap_or_else(|| "{}".into()),
            })
            .collect()
    }
}

#[derive(Deserialize)]
struct WireUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
    #[serde(default)]
    #[serde(rename = "prompt_tokens_details")]
    prompt_tokens_details: Option<PromptTokensDetails>,
    /// The cost in credits, which OpenRouter prices in dollars.
    #[serde(default)]
    cost: Option<f64>,
}

#[derive(Deserialize)]
struct PromptTokensDetails {
    #[serde(default, rename = "cached_tokens")]
    cached_tokens: u64,
}

#[derive(Deserialize)]
struct WireError {
    error: WireErrorBody,
}

#[derive(Deserialize)]
struct WireErrorBody {
    message: String,
}

impl ModelProvider for OpenRouter {
    fn complete(&self, request: CompletionRequest) -> CompletionFuture<'_> {
        Box::pin(self.post(request))
    }
}

impl OpenRouter {
    /// One POST to `chat/completions`, with the statuses mapped to the provider errors.
    async fn post(&self, request: CompletionRequest) -> Result<Completion, ProviderError> {
        let wire = WireRequest {
            model: &request.model,
            messages: request.messages.iter().map(|m| to_wire(m)).collect(),
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            tools: request.tools.iter().map(|t| to_wire_tool(t)).collect(),
        };
        let url = format!("{}/chat/completions", self.base_url);
        let mut post = self.client.post(&url).json(&wire);
        if !self.is_local() {
            post = post.bearer_auth(&self.key);
        }
        // the URL is in the error on purpose: a local server that is not running is the usual
        // cause, and the message says where the console looked
        let response = post.send().await.map_err(|e| ProviderError::Unavailable(format!("cannot reach {url}: {e}")))?;
        let status = response.status();
        let body = response.text().await.map_err(|e| ProviderError::Unavailable(format!("{url} dropped the answer: {e}")))?;
        match status {
            StatusCode::OK => {
                let parsed: WireResponse = serde_json::from_str(&body)
                    .map_err(|e| ProviderError::Other(format!("unparsable answer: {e}; body starts with: {}", snippet(&body))))?;
                let choice = parsed
                    .choices
                    .first()
                    .ok_or_else(|| ProviderError::Other(format!("the answer holds no choices; body starts with: {}", snippet(&body))))?;
                let tool_calls = choice.message.tool_calls();
                let text = choice.message.text();
                if tool_calls.is_empty() && text.trim().is_empty() {
                    return Err(ProviderError::Other(format!(
                        "the answer holds no text and no tool calls; body starts with: {}",
                        snippet(&body)
                    )));
                }
                let usage = parsed
                    .usage
                    .map(|u| Usage {
                        prompt_tokens: u.prompt_tokens,
                        completion_tokens: u.completion_tokens,
                        cached_tokens: u.prompt_tokens_details.map_or(0, |d| d.cached_tokens),
                        cost_usd: u.cost,
                    })
                    .unwrap_or_default();
                Ok(Completion {
                    text,
                    usage,
                    turns: 1,
                    turn_usage: vec![crate::TurnUsage::of(&usage)],
                    tool_calls,
                    session_id: None,
                    gotchas: Vec::new(),
                })
            }
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(ProviderError::Auth(error_text(&body, status))),
            StatusCode::NOT_FOUND => Err(ProviderError::UnknownModel(self.unknown_model_text(&body, status).await)),
            StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE | StatusCode::BAD_GATEWAY => {
                Err(ProviderError::Unavailable(error_text(&body, status)))
            }
            _ => Err(ProviderError::Other(error_text(&body, status))),
        }
    }
}

/// The message of the error body, or the status text when the body is not the usual shape.
fn error_text(body: &str, status: StatusCode) -> String {
    serde_json::from_str::<WireError>(body).ok().map(|e| e.error.message).unwrap_or_else(|| format!("HTTP {status}: {body}"))
}

/// The first 200 characters of a body, for an error message.
fn snippet(body: &str) -> String {
    let mut text: String = body.chars().take(200).collect();
    if body.chars().count() > 200 {
        text.push('…');
    }
    text
}

/// Maps one message of the conversation onto the wire. A tool message sends `content` even when it
/// is empty; an assistant message that asks for tool calls sends `content` only when it has text,
/// since the OpenRouter validation rejects an empty string next to tool calls.
fn to_wire(message: &Message) -> WireMessage<'_> {
    let empty_beside_calls = !message.tool_calls.is_empty() && message.content.trim().is_empty();
    WireMessage {
        role: &message.role,
        content: (!empty_beside_calls).then_some(message.content.as_str()),
        tool_calls: message.tool_calls.iter().map(|c| to_wire_call(c)).collect(),
        answering: message.tool_call_id.as_deref(),
    }
}

fn to_wire_call(call: &ToolCall) -> WireToolCall<'_> {
    WireToolCall { id: &call.id, kind: "function", function: WireFunction { name: &call.name, arguments: &call.arguments } }
}

fn to_wire_tool(spec: &ToolSpec) -> WireTool<'_> {
    WireTool {
        kind: "function",
        function: WireToolFunction { name: &spec.name, description: &spec.description, parameters: &spec.parameters },
    }
}

#[cfg(test)]
mod tests;
