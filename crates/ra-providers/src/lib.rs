//! The provider abstraction. A provider turns a chat request into a completion, whatever the
//! vendor behind it. The orchestrator and the workers know only [`ModelProvider`]; swapping
//! GLM for Claude or a local model changes no caller.
//!
//! Two adapters so far: Claude Code, the local harness on a Claude subscription, and OpenRouter,
//! the aggregator for every other vendor. The OpenRouter adapter also serves a local
//! OpenAI-compatible server such as llama.cpp, with another base URL and no key.

pub mod claude_code;
pub mod limits;
pub mod local;
pub mod openrouter;

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

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
    /// The directory the worker may act in. `None` is a pure chat completion; a provider with
    /// tools of its own, such as Claude Code, runs them inside this directory and nowhere else.
    pub workspace: Option<PathBuf>,
    /// The most dollars the run may spend; `None` sets no cap. A harness such as Claude Code
    /// enforces it itself; for a bare model the caller's tool loop does.
    pub budget_usd: Option<f64>,
    /// The harness session to resume, so a later run of the same task keeps its context; a
    /// provider without sessions ignores it.
    pub resume: Option<String>,
    /// The most turns a worker run may take; `None` leaves the provider's own cap. A harness
    /// enforces it itself; for a bare model the caller's tool loop does. It stays on when the
    /// dollar budget is off: on a subscription the turns are the cost.
    pub max_turns: Option<u32>,
    /// The effort the run asks of the model, `low`, `medium` or `high`; `None` leaves the
    /// provider's own setting. A harness with the knob passes it on; a bare model ignores it.
    pub effort: Option<String>,
}

/// The turn cap of a run on a node of `complexity`, when nothing names one: c1 15, c2 25, c3 40,
/// c4 60, c5 90. An out-of-range complexity takes the nearest end.
pub fn turns_for(complexity: u8) -> u32 {
    match complexity {
        0 | 1 => 15,
        2 => 25,
        3 => 40,
        4 => 60,
        _ => 90,
    }
}

/// The share of the plan's five-hour window left under which the window is tight: a walk then
/// takes the smallest ready card first, so what is left of the window finishes cards whole
/// rather than starting a large one it cannot end.
pub const TIGHT_SHARE: f64 = 0.25;

/// One answer, with the usage the provider reported.
#[derive(Debug, Clone)]
pub struct Completion {
    pub text: String,
    pub usage: Usage,
    /// The turns the run took, as a harness reports them; one from a provider that answers a
    /// single completion.
    pub turns: u64,
    /// The tokens of each assistant turn, in order, as the provider showed them go by; one
    /// entry from a provider that answers a single completion, none from one that cannot tell.
    pub turn_usage: Vec<TurnUsage>,
    /// The tool calls the answer asks for; empty means a plain answer.
    pub tool_calls: Vec<ToolCall>,
    /// The harness session the answer came from, for a later run to resume; `None` from a
    /// provider without sessions.
    pub session_id: Option<String>,
    /// The gotchas the run hit, as the provider saw them go by: the commands the machine refused,
    /// the errors it knows, the claims the answer made. Empty from a provider that does not look.
    pub gotchas: Vec<ra_gotchas::Hit>,
}

/// The tokens of one assistant turn: what it read, cached or not, and what it wrote.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TurnUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl TurnUsage {
    /// The one turn of a single completion.
    #[must_use]
    pub fn of(usage: &Usage) -> Self {
        Self { input_tokens: usage.prompt_tokens, output_tokens: usage.completion_tokens }
    }

    /// Both sides together, the size of the turn.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

/// The token counts of one execution, as the provider reported them.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// The tokens served from the provider cache, when it reports them.
    pub cached_tokens: u64,
    /// The cost in dollars as the provider reported it; `None` when it reports none. On a
    /// subscription this is the list price of the call, not a charge.
    pub cost_usd: Option<f64>,
    /// Whether the cost was computed here from the model's list prices because the provider
    /// reported none; a local model's zero is computed too. The ledger marks such a row.
    #[serde(default)]
    pub cost_computed: bool,
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

/// The future of one completion, boxed so a provider can stand behind `dyn ModelProvider`.
pub type CompletionFuture<'a> = Pin<Box<dyn Future<Output = Result<Completion, ProviderError>> + Send + 'a>>;

/// The one interface every provider implements. It is object-safe on purpose: the router hands
/// out `Arc<dyn ModelProvider>` and the workers never learn the vendor.
pub trait ModelProvider: Send + Sync {
    /// Runs one completion.
    fn complete(&self, request: CompletionRequest) -> CompletionFuture<'_>;

    /// Whether the provider is a harness with tools of its own, which acts in the request's
    /// workspace by itself. A bare model answers `false`, and the caller runs the tool loop.
    fn has_own_tools(&self) -> bool {
        false
    }

    /// The command line a harness would run for `request`, binary first, for a dry run to show;
    /// `None` from a provider that makes a request rather than running a command.
    fn command_line(&self, _request: &CompletionRequest) -> Option<Vec<String>> {
        None
    }
}
