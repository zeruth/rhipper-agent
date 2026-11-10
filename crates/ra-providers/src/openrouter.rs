//! The OpenRouter adapter: one HTTP POST to `chat/completions` per completion. It maps the HTTP
//! statuses to the provider errors so a caller can retry the transient ones.

use reqwest::StatusCode;
use serde::Deserialize;

use crate::{Completion, CompletionRequest, Message, ModelProvider, ProviderError, Usage};

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

/// The wire format of one chat message.
#[derive(serde::Serialize)]
struct WireMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(serde::Serialize)]
struct WireRequest<'a> {
    model: &'a str,
    messages: Vec<WireMessage<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
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
}

impl WireMessageOwned {
    /// The text of the answer: the content, else the reasoning.
    fn text(&self) -> String {
        self.content.clone().filter(|c| !c.trim().is_empty()).or_else(|| self.reasoning.clone()).unwrap_or_default()
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
            messages: request.messages.iter().map(|Message { role, content }| WireMessage { role, content }).collect(),
            temperature: request.temperature,
            max_tokens: request.max_tokens,
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
                let text = parsed
                    .choices
                    .first()
                    .map(|c| c.message.text())
                    .filter(|t| !t.is_empty())
                    .ok_or_else(|| ProviderError::Other(format!("the answer holds no text; body starts with: {}", snippet(&body))))?;
                let usage = parsed
                    .usage
                    .map(|u| Usage {
                        prompt_tokens: u.prompt_tokens,
                        completion_tokens: u.completion_tokens,
                        cached_tokens: u.prompt_tokens_details.map_or(0, |d| d.cached_tokens),
                    })
                    .unwrap_or_default();
                Ok(Completion { text, usage })
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

#[cfg(test)]
mod tests;
