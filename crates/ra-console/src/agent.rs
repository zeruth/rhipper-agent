//! The agent loop: the console's live path to a model. `ping` proves the connection; `run` sends
//! one roadmap task to the badge model and records what came back.

use std::collections::HashMap;

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
            tools: Vec::new(),
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
            tools: Vec::new(),
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
    let nodes = store.all().await?;
    // one pass over the tree before any call, so the recursion cannot stall midway
    let order = if recursive { subtree_preorder(&nodes, id) } else { vec![id] };
    let mut revised: Vec<String> = Vec::new();
    let mut verified: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut receipts: Vec<String> = Vec::new();
    for current in order {
        let Some(node) = nodes.iter().find(|n| n.id == current) else {
            warnings.push(format!("⚠ #{}: no such node", current));
            continue;
        };
        // a node whose work is done is verified, not recreated: no model call, no chance to drift
        if matches!(node.state, TaskState::Implemented | TaskState::Accepted) {
            receipts.push(format!("#{} {} [{}] verified: no revision needed", node.id, node.title, node.state.serde_name()));
            verified.push(format!("#{}", current));
            continue;
        }
        match reassess_one(store, &provider, model, current, &nodes).await {
            Ok(receipt) => {
                revised.push(format!("#{}", current));
                receipts.push(receipt);
            }
            Err(e) => warnings.push(format!("⚠ #{}: {e:#}", current)),
        }
    }
    let mut out = vec![format!(
        "✓ {} node(s) reassessed on {} ({} verified): {}",
        revised.len() + verified.len(),
        short(model),
        verified.len(),
        revised.join(" ")
    )];
    out.extend(receipts);
    out.extend(warnings);
    Ok(out)
}

/// The subtree of `root` in preorder: the node first, then each child's subtree in id order.
pub fn subtree_preorder(nodes: &[Node], root: i64) -> Vec<i64> {
    let mut kids: HashMap<i64, Vec<i64>> = HashMap::new();
    for node in nodes {
        if let Some(parent) = node.parent {
            kids.entry(parent).or_default().push(node.id);
        }
    }
    for list in kids.values_mut() {
        list.sort_unstable();
    }
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        out.push(id);
        if let Some(children) = kids.get(&id) {
            for child in children.iter().rev() {
                stack.push(*child);
            }
        }
    }
    out
}

/// One reassessment: the model returns two lines, and the store takes them. The prompt carries the
/// node's own notes and its children with their states, so the revision names only the work the
/// subtree still owes.
async fn reassess_one(
    store: &Store,
    provider: &ra_providers::openrouter::OpenRouter,
    model: &str,
    id: i64,
    nodes: &[Node],
) -> anyhow::Result<String> {
    let node = store.read(id).await?;
    let old_objective = node.objective.clone();
    let old_acceptance = node.acceptance.clone();
    let context = context_of(store, &node).await?;
    let mut ask = context;
    if let Some(notes) = &node.notes {
        ask.push_str(&format!("\nNotes of earlier work on this node: {}\n", first_line(notes)));
    }
    let children: Vec<&Node> = nodes.iter().filter(|n| n.parent == Some(id)).collect();
    if !children.is_empty() {
        ask.push_str("\nWhat this node's children already cover:\n");
        for kid in children {
            ask.push_str(&format!(
                "- #{} {} [{}]: {}\n",
                kid.id,
                kid.title,
                kid.state.serde_name(),
                kid.objective.as_deref().unwrap_or("no objective")
            ));
        }
    }
    ask.push_str(
        "\nRevise the objective and the acceptance so they name only the work still ahead: leave out what the \
         children and the notes show as done, and make every acceptance criterion concrete and checkable without \
         interpretation. Name the exact command or observable behavior that proves each criterion. Never write \
         placeholder text in angle brackets: write the actual revised words. Reply with exactly three lines and \
         nothing else:\n\
         overview: <one sentence on what you changed and why>\n\
         objective: <the revised objective, one line>\nacceptance: <the revised acceptance, one line>",
    );
    let answer = provider
        .complete(CompletionRequest {
            model: model.to_owned(),
            messages: vec![Message::system(ARCHITECT), Message::user(ask)],
            temperature: Some(0.2),
            max_tokens: Some(512),
            tools: Vec::new(),
        })
        .await
        .map_err(map_error)?;
    let mut overview: Option<String> = None;
    let mut objective: Option<String> = None;
    let mut acceptance: Option<String> = None;
    for line in answer.text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("overview:") {
            if !rest.trim().is_empty() {
                overview = Some(rest.trim().to_owned());
            }
        } else if let Some(rest) = line.strip_prefix("objective:") {
            let rest = rest.trim();
            if !rest.is_empty() && !is_placeholder(rest) {
                objective = Some(rest.to_owned());
            }
        } else if let Some(rest) = line.strip_prefix("acceptance:") {
            let rest = rest.trim();
            if !rest.is_empty() && !is_placeholder(rest) {
                acceptance = Some(rest.trim_start().to_owned());
            }
        }
    }
    anyhow::ensure!(
        objective.is_some() || acceptance.is_some(),
        "the reassessment held no real objective or acceptance; the answer started with: {}",
        answer.text.lines().next().unwrap_or("")
    );
    store.set_details(id, objective.as_deref(), acceptance.as_deref()).await?;

    // the receipt reads like a patch: an overview line, then the old and the new of each field
    let mut receipt = format!("#{} {}", node.id, node.title);
    if let Some(overview) = &overview {
        receipt.push_str(&format!("\n~ overview: {overview}"));
    }
    let mut changed = 0usize;
    for (name, old, new) in [("objective", old_objective, objective.clone()), ("acceptance", old_acceptance, acceptance.clone())] {
        match (old, new) {
            (Some(old), Some(new)) if old != new => {
                receipt.push_str(&format!("\n- {name}: {old}"));
                receipt.push_str(&format!("\n+ {name}: {new}"));
                changed += 1;
            }
            (Some(_), Some(_)) => receipt.push_str(&format!("\n~ {name}: unchanged")),
            (None, Some(new)) => {
                receipt.push_str(&format!("\n+ {name}: {new}"));
                changed += 1;
            }
            (Some(_), None) => {
                receipt.push_str(&format!("\n- {name}: removed"));
                changed += 1;
            }
            (None, None) => {}
        }
    }
    receipt.push_str(&format!("\n{changed} field(s) changed"));
    Ok(receipt)
}

/// Whether a revised field is template text the model echoed back, such as `<the revised
/// objective, one line>`: it opens and closes with angle brackets, so it names no real work.
fn is_placeholder(value: &str) -> bool {
    value.starts_with('<') && value.ends_with('>')
}

/// The first line of a note, for a prompt that stays short.
fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").chars().take(200).collect()
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
