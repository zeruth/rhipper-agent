//! The provider abstraction. A provider turns a chat request into a completion, whatever the
//! vendor behind it. The orchestrator and the workers know only [`ModelProvider`]; swapping
//! GLM for Claude or a local model changes no caller.
//!
//! The OpenRouter adapter is the first provider and the aggregator for the others.

pub mod openrouter;

use std::future::Future;

use serde::Deserialize;

/// One message of a chat conversation.
#[derive(Debug, Clone)]
pub struct Message {
    /// `system`, `user`, `assistant` or `tool`.
    pub role: String,
    pub content: String,
    /// The tool calls of an assistant message.
    pub tool_calls: Vec<ToolCall>,
    /// The id of the call a tool message answers.
    pub tool_call_id: Option<String>,
}

/// One tool call an assistant asked for.
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// The arguments, as a JSON object in text form.
    pub arguments: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: "system".into(), content: content.into(), tool_calls: Vec::new(), tool_call_id: None }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self { role: "user".into(), content: content.into(), tool_calls: Vec::new(), tool_call_id: None }
    }

    /// An assistant message that asks for tool calls.
    pub fn assistant_calls(content: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self { role: "assistant".into(), content: content.into(), tool_calls, tool_call_id: None }
    }

    /// The result of one tool call.
    pub fn tool(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self { role: "tool".into(), content: content.into(), tool_calls: Vec::new(), tool_call_id: Some(call_id.into()) }
    }
}

/// A tool the model may call, with a JSON schema for its arguments.
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// A request for one completion.
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    /// The temperature; `None` uses the provider default.
    pub temperature: Option<f64>,
    /// The maximum tokens of the answer; `None` lets the provider decide.
    pub max_tokens: Option<u32>,
    /// The tools the model may call; empty sends no tools.
    pub tools: Vec<ToolSpec>,
}

/// One answer, with the usage the provider reported.
#[derive(Debug, Clone)]
pub struct Completion {
    pub text: String,
    pub usage: Usage,
    /// The tool calls the answer asks for; empty means a plain answer.
    pub tool_calls: Vec<ToolCall>,
}

/// The token counts of one execution, as the provider reported them.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// The tokens served from the provider cache, when it reports them.
    pub cached_tokens: u64,
}

/// The errors of a provider call, with the ones a caller can retry marked.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// The provider refused the key or the organization.
    #[error("authentication failed: {0}")]
    Auth(String),
    /// The model id is unknown to the provider.
    #[error("unknown model {0}")]
    UnknownModel(String),
    /// The provider is rate-limiting or unreachable; retry later.
    #[error("provider unavailable: {0}")]
    Unavailable(String),
    /// Anything else.
    #[error("{0}")]
    Other(String),
}

/// The one interface every provider implements.
pub trait ModelProvider: Send + Sync {
    /// Runs one completion.
    fn complete(&self, request: CompletionRequest) -> impl Future<Output = Result<Completion, ProviderError>> + Send;
}
