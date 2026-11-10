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
        format!(
            "✓ #{} implemented on {} · in {} out {}",
            node.title,
            short(model),
            answer.usage.prompt_tokens,
            answer.usage.completion_tokens
        ),
        "  the full answer is in the notes of the card in rhipper.md".to_owned(),
    ])
}

/// Reassesses one node: the model revises the objective and the acceptance for the work still
/// ahead, from the node's current text and its notes. With `recursive`, the whole subtree follows,
/// parent first.
pub async fn reassess(store: &Store, id: i64, model: &str, recursive: bool) -> anyhow::Result<Vec<String>> {
    let key = api_key()?;
    let provider = ra_providers::openrouter::OpenRouter::new(key)?;
    let mut revised: Vec<String> = Vec::new();
    let mut queue = vec![id];
    while let Some(current) = queue.pop() {
        reassess_one(store, &provider, model, current).await?;
        revised.push(format!("#{}", current));
        if recursive {
            let kids: Vec<i64> = store.all().await?.iter().filter(|n| n.parent == Some(current)).map(|n| n.id).collect();
            // the children go back on the stack, so the parent reassesses first
            queue.extend(kids);
        }
    }
    Ok(vec![format!("✓ {} node(s) reassessed on {}: {}", revised.len(), short(model), revised.join(" "))])
}

/// One reassessment: the model returns two lines, and the store takes them.
async fn reassess_one(store: &Store, provider: &ra_providers::openrouter::OpenRouter, model: &str, id: i64) -> anyhow::Result<Vec<String>> {
    let node = store.read(id).await?;
    let context = context_of(store, &node).await?;
    let ask = format!(
        "{context}\n\nThe task above may have drifted from the work still ahead. Revise its objective and its \
         acceptance so they name the remaining work alone. Reply with exactly two lines and nothing else:\n\
         objective: <the revised objective, one line>\nacceptance: <the revised acceptance, one line>"
    );
    let answer = provider
        .complete(CompletionRequest {
            model: model.to_owned(),
            messages: vec![Message::system(ARCHITECT), Message::user(ask)],
            temperature: Some(0.2),
            max_tokens: Some(256),
        })
        .await
        .map_err(map_error)?;
    let mut objective = None;
    let mut acceptance = None;
    for line in answer.text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("objective:") {
            if !rest.trim().is_empty() {
                objective = Some(rest.trim().to_owned());
            }
        } else if let Some(rest) = line.strip_prefix("acceptance:") {
            if !rest.trim().is_empty() {
                acceptance = Some(rest.trim().to_owned());
            }
        }
    }
    anyhow::ensure!(
        objective.is_some() || acceptance.is_some(),
        "#{}: the reassessment held no objective or acceptance line; the answer started with: {}",
        id,
        answer.text.lines().next().unwrap_or("")
    );
    store.set_details(id, objective.as_deref(), acceptance.as_deref()).await?;
    Ok(vec![format!("#{} {} revised", node.id, node.title)])
}

/// The system prompt of a worker session.
const SYSTEM: &str = "You are an implementation worker of a roadmap-driven orchestrator. You receive one task with its objective, acceptance criteria and related context. You answer with a plan of the implementation and the code or the changes it names, nothing else.";

/// The system prompt of a reassessment.
const ARCHITECT: &str = "You are the architect of a roadmap-driven orchestrator. You receive one roadmap node with its context. You keep the objective and the acceptance short, one topic per sentence, and you name the remaining work, not the work already done.";

/// The prompt of one task: the node, its ancestors, and its dependencies.
async fn context_of(store: &Store, node: &Node) -> anyhow::Result<String> {
    let mut text = format!("Task #{}: {}\n", node.id, node.title);
    if let Some(objective) = &node.objective {
        text.push_str(&format!("Objective: {objective}\n"));
    }
    if let Some(acceptance) = &node.acceptance {
        text.push_str(&format!("Acceptance criteria: {acceptance}\n"));
    }
    // the ancestors: the chain of parents above the task, nearest first
    let mut parent = node.parent;
    while let Some(id) = parent {
        let ancestor = store.read(id).await?;
        text.push_str(&format!("Under #{} {}: {}\n", ancestor.id, ancestor.title, ancestor.objective.as_deref().unwrap_or("no objective")));
        parent = ancestor.parent;
    }
    // the dependencies, with their own context and the notes of earlier attempts
    for id in &node.depends_on {
        if let Ok(dep) = store.read(*id).await {
            text.push_str(&format!(
                "Depends on #{} {} [{}]: {}\n",
                dep.id,
                dep.title,
                dep.state.serde_name(),
                dep.objective.as_deref().unwrap_or("no objective")
            ));
            if let Some(notes) = &dep.notes {
                text.push_str(&format!("  notes of #{}: {notes}\n", dep.id));
            }
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
