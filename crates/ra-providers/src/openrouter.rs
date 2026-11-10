//! The OpenRouter adapter: one HTTP POST to `chat/completions` per completion. It maps the HTTP
//! statuses to the provider errors so a caller can retry the transient ones.

use reqwest::StatusCode;
use serde::Deserialize;

use crate::{Completion, CompletionRequest, Message, ModelProvider, ProviderError, ToolCall, ToolSpec, Usage};

/// The OpenRouter provider.
pub struct OpenRouter {
    client: reqwest::Client,
    key: String,
    /// The API root; the tests point it at a local server.
    base_url: String,
}

impl OpenRouter {
    /// Builds the provider from an OpenRouter API key.
    pub fn new(key: impl Into<String>) -> anyhow::Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder().user_agent("rhipper-cmd/0.1").build()?,
            key: key.into(),
            base_url: "https://openrouter.ai/api/v1".into(),
        })
    }

    /// Builds the provider against another base URL, for tests.
    pub fn at(base_url: &str, key: impl Into<String>) -> anyhow::Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder().user_agent("rhipper-cmd/0.1").build()?,
            key: key.into(),
            base_url: base_url.to_owned(),
        })
    }
}

/// The wire format of one chat message. Content is optional because tool messages may carry it
/// as null and reasoning models answer in `reasoning` instead.
#[derive(serde::Serialize, Deserialize)]
struct WireMessage<'a> {
    role: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<&'a str>,
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
    async fn complete(&self, request: CompletionRequest) -> Result<Completion, ProviderError> {
        let wire = WireRequest {
            model: &request.model,
            messages: request.messages.iter().map(|m| to_wire(m)).collect(),
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            tools: request.tools.iter().map(|t| to_wire_tool(t)).collect(),
        };
        let url = format!("{}/chat/completions", self.base_url);
        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.key)
            .json(&wire)
            .send()
            .await
            .map_err(|e| ProviderError::Unavailable(e.to_string()))?;
        let status = response.status();
        let body = response.text().await.map_err(|e| ProviderError::Unavailable(e.to_string()))?;
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
                    })
                    .unwrap_or_default();
                Ok(Completion { text, usage, tool_calls })
            }
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(ProviderError::Auth(error_text(&body, status))),
            StatusCode::NOT_FOUND => Err(ProviderError::UnknownModel(error_text(&body, status))),
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
/// is empty; an assistant message sends `content` only when it has text, since the OpenRouter
/// validation rejects an empty string next to tool calls.
fn to_wire(message: &Message) -> WireMessage<'_> {
    WireMessage {
        role: &message.role,
        content: Some(message.content.as_str()),
        tool_calls: message.tool_calls.iter().map(|c| to_wire_call(c)).collect(),
        answering: message.tool_call_id.as_deref(),
    }
}

fn to_wire_call(call: &ToolCall) -> WireToolCall<'_> {
    WireToolCall {
        id: &call.id,
        kind: "function",
        function: WireFunction { name: &call.name, arguments: &call.arguments },
    }
}

fn to_wire_tool(spec: &ToolSpec) -> WireTool<'_> {
    WireTool {
        kind: "function",
        function: WireToolFunction { name: &spec.name, description: &spec.description, parameters: &spec.parameters },
    }
}

#[cfg(test)]
mod tests;
