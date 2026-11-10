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
    /// `system`, `user` or `assistant`.
    pub role: String,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: "system".into(), content: content.into() }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self { role: "user".into(), content: content.into() }
    }
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
}

/// One answer, with the usage the provider reported.
#[derive(Debug, Clone)]
pub struct Completion {
    pub text: String,
    pub usage: Usage,
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
