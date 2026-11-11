//! The local route's cheap roles. A run ends with work no frontier model needs: the summary of
//! the worker's report for the handoff, the one-line digest of what the run did for the other
//! projects, and whether a look-back bullet the worker left untagged is about the machine. Each
//! is a [`Role`]: one short completion with no tools, booked under its own model so the saving
//! shows in the ledger. The `[role]` table of `rhipper.toml` names a model for each; without one
//! the local server at `RHIPPER_LOCAL_URL` takes the role when it answers a ping, and nothing is
//! called when it does not.

use std::time::{Duration, Instant};

use crate::openrouter::{OpenRouter, local_url};
use crate::{CompletionRequest, Message, ModelProvider, ProviderError, Usage};

/// How long the one-token probe and a role's call wait for the answer: a server with the model
/// loaded answers in well under a second, one still loading it in a few; past this it is not
/// serving.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(30);

/// The cheap roles a run hands off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The worker's report condensed for the next run of the card.
    Summary,
    /// What the run did, in one line, for the other projects' digest.
    Digest,
    /// Whether an untagged look-back bullet is about this machine.
    Classify,
}

impl Role {
    /// Every role, in the order a run asks them.
    pub const ALL: [Self; 3] = [Self::Summary, Self::Digest, Self::Classify];

    /// The key of the role in the `[role]` table, and its name in a log line.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Summary => "summary",
            Self::Digest => "digest",
            Self::Classify => "classify",
        }
    }

    /// The most tokens the role's answer takes: a short text, a line, a word.
    #[must_use]
    pub fn room(self) -> u32 {
        match self {
            Self::Summary => 400,
            Self::Digest => 80,
            Self::Classify => 8,
        }
    }
}

/// Whether the local server answers a ping: its model list within the probe's timeout.
pub async fn up() -> bool {
    up_at(&local_url()).await
}

/// [`up`] against the server at `base_url`.
pub async fn up_at(base_url: &str) -> bool {
    default_model_at(base_url).await.is_some()
}

/// The model the roles default to: `local/<the first id the server lists>`, or `local/default`
/// when the server answers with an empty list; `None` when nothing answers at [`local_url`].
pub async fn default_model() -> Option<String> {
    default_model_at(&local_url()).await
}

/// [`default_model`] against the server at `base_url`.
pub async fn default_model_at(base_url: &str) -> Option<String> {
    let models = OpenRouter::at(base_url, "").ok()?.models().await.ok()?;
    Some(format!("local/{}", models.first().map_or("default", String::as_str)))
}

/// What the one-token probe found: the model that answered, as the server lists it, and the
/// round trip of the completion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    pub model: String,
    pub latency: Duration,
}

/// [`probe_at`] against the server at [`local_url`].
///
/// # Errors
/// The reason the server did not answer.
pub async fn probe() -> Result<Probe, String> {
    probe_at(&local_url()).await
}

/// Whether the server at `base_url` serves, not only lists: the first model it names is asked
/// for one token, timed from the request to the answer. A ping says a process listens; this
/// says a model completes, which is what the roles need. The error is the refusal's reason as
/// the doctor prints it: nothing listening, no model loaded, or what the completion said.
///
/// # Errors
/// The reason the server did not answer.
pub async fn probe_at(base_url: &str) -> Result<Probe, String> {
    let server = OpenRouter::at(base_url, "").map_err(|e| format!("no client for {base_url}: {e:#}"))?;
    let models = server.models().await.map_err(|e| e.to_string())?;
    let model = models.into_iter().next().ok_or_else(|| format!("{base_url} answers but lists no model"))?;
    let request = CompletionRequest {
        model: model.clone(),
        messages: vec![Message::user("Say ok.")],
        temperature: Some(0.0),
        max_tokens: Some(1),
        tools: Vec::new(),
        workspace: None,
        budget_usd: None,
        resume: None,
        max_turns: Some(1),
        effort: None,
    };
    let started = Instant::now();
    match tokio::time::timeout(ANSWER_TIMEOUT, server.complete(request)).await {
        Ok(Ok(_)) => Ok(Probe { model, latency: started.elapsed() }),
        Ok(Err(e)) => Err(format!("{model} refused the one-token request: {e}")),
        Err(_) => Err(format!("{model} gave no token in {}s", ANSWER_TIMEOUT.as_secs())),
    }
}

/// One call of `role` on `model` through `provider`: `text` under `instruction`, at temperature
/// zero and within the role's room, with the answer trimmed. A server that neither answers nor
/// refuses within [`ANSWER_TIMEOUT`] is unavailable, like one that is not listening: the run
/// takes its rule-based path rather than waiting on it.
///
/// # Errors
/// The provider's own, or [`ProviderError::Unavailable`] when the answer timed out.
pub async fn ask(
    provider: &dyn ModelProvider,
    model: &str,
    role: Role,
    instruction: &str,
    text: &str,
) -> Result<(String, Usage), ProviderError> {
    let request = CompletionRequest {
        model: model.to_owned(),
        messages: vec![Message::system(instruction), Message::user(text)],
        temperature: Some(0.0),
        max_tokens: Some(role.room()),
        tools: Vec::new(),
        workspace: None,
        budget_usd: None,
        resume: None,
        max_turns: Some(1),
        effort: Some("low".to_owned()),
    };
    let answer = tokio::time::timeout(ANSWER_TIMEOUT, provider.complete(request))
        .await
        .map_err(|_| ProviderError::Unavailable(format!("{model} gave no answer in {}s", ANSWER_TIMEOUT.as_secs())))??;
    Ok((answer.text.trim().to_owned(), answer.usage))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// A server that answers one request with `body`.
    fn serve(body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut socket, _)) = listener.accept() {
                let mut buffer = [0u8; 4096];
                let _ = socket.read(&mut buffer);
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes());
            }
        });
        format!("http://127.0.0.1:{port}/v1")
    }

    #[tokio::test]
    async fn the_roles_default_to_the_first_local_model_when_the_server_answers_and_to_none_when_it_does_not() {
        let url = serve(r#"{"object":"list","data":[{"id":"qwen3-8b"},{"id":"llama-3.2-3b"}]}"#);
        assert_eq!(default_model_at(&url).await.as_deref(), Some("local/qwen3-8b"));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let closed = format!("http://127.0.0.1:{}/v1", listener.local_addr().unwrap().port());
        drop(listener);
        assert_eq!(default_model_at(&closed).await, None);
        assert!(!up_at(&closed).await);
        assert_eq!(Role::ALL.map(Role::name), ["summary", "digest", "classify"]);
    }
}
