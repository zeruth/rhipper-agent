//! The agent loop: the console's live path to a model. `ping` proves the connection; `run` sends
//! one roadmap task to the badge model and records what came back.

use crate::secret;
use ra_providers::{CompletionRequest, Message, ModelProvider, ProviderError};
use ra_roadmap::model::{Node, TaskState};
use ra_roadmap::store::Store;

/// The model of the next run, as the badge holds it.
pub type Model = String;

/// Reads the OpenRouter key: the credential store, with the environment as the override.
pub fn api_key() -> anyhow::Result<String> {
    secret::api_key()
}

/// Sends one message and returns the answer and the usage as log lines.
pub async fn ping(model: &str) -> anyhow::Result<Vec<String>> {
    let key = api_key()?;
    let provider = ra_providers::openrouter::OpenRouter::new(key)?;
    let answer = provider
        .complete(CompletionRequest {
            model: model.to_owned(),
            messages: vec![Message::user("Reply with the single word: pong.")],
            temperature: Some(0.0),
            max_tokens: Some(16),
        })
        .await
        .map_err(map_error)?;
    Ok(vec![
        format!("{} says: {}", short(model), answer.text.trim()),
        format!("  tokens: in {} out {} cached {}", answer.usage.prompt_tokens, answer.usage.completion_tokens, answer.usage.cached_tokens),
    ])
}

/// Runs one roadmap task: builds the context from the node, its ancestors and its dependencies,
/// sends it to the model, and writes the answer and the state back.
pub async fn run(store: &Store, id: i64, model: &str) -> anyhow::Result<Vec<String>> {
    // the node first, so a bad id or a project is named before the key matters
    let node = store.read(id).await?;
    anyhow::ensure!(node.kind != ra_roadmap::model::NodeKind::Project, "#{} is a project; run a task", id);
    let key = api_key()?;
    store.set_state(id, TaskState::InProgress).await?;

    let context = context_of(store, &node).await?;
    let provider = ra_providers::openrouter::OpenRouter::new(key)?;
    let answer = provider
        .complete(CompletionRequest {
            model: model.to_owned(),
            messages: vec![Message::system(SYSTEM), Message::user(context)],
            temperature: Some(0.4),
            max_tokens: Some(1024),
        })
        .await
        .map_err(map_error)?;

    store.set_notes(id, &answer.text).await?;
    store.set_state(id, TaskState::Implemented).await?;
    Ok(vec![
        format!("#{} implemented on {}", node.title, short(model)),
        format!("  in {} out {} cached {}", answer.usage.prompt_tokens, answer.usage.completion_tokens, answer.usage.cached_tokens),
        answer.text,
    ])
}

/// The system prompt of a worker session.
const SYSTEM: &str = "You are an implementation worker of a roadmap-driven orchestrator. You receive one task with its objective, acceptance criteria and related context. You answer with a plan of the implementation and the code or the changes it names, nothing else.";

/// The prompt of one task: the node, its ancestors and its dependencies.
async fn context_of(store: &Store, node: &Node) -> anyhow::Result<String> {
    let mut text = format!("Task #{}: {}\n", node.id, node.title);
    if let Some(objective) = &node.objective {
        text.push_str(&format!("Objective: {objective}\n"));
    }
    if let Some(acceptance) = &node.acceptance {
        text.push_str(&format!("Acceptance criteria: {acceptance}\n"));
    }
    for id in &node.depends_on {
        if let Ok(dep) = store.read(*id).await {
            text.push_str(&format!("Depends on #{} {}: {}\n", dep.id, dep.title, dep.state.serde_name()));
        }
    }
    Ok(text)
}

/// Maps a provider error to a refusal that names what to do.
fn map_error(error: ProviderError) -> anyhow::Error {
    match error {
        ProviderError::Auth(_) => anyhow::anyhow!("OpenRouter refused the key: check OPENROUTER_API_KEY"),
        ProviderError::UnknownModel(model) => anyhow::anyhow!("OpenRouter knows no model {model}"),
        ProviderError::Unavailable(text) => anyhow::anyhow!("OpenRouter is unreachable: {text}"),
        ProviderError::Other(text) => anyhow::anyhow!("the run failed: {text}"),
    }
}

/// The model id without its vendor.
fn short(model: &str) -> &str {
    model.rsplit('/').next().unwrap_or(model)
}
