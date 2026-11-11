//! The agent loop: the console's live path to a model. `ping` proves the connection; `run` sends
//! one roadmap task to the badge model and records what came back. The functions talk to a
//! [`ModelProvider`] through a [`Worker`], never to a vendor.

use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use parking_lot::Mutex;
use ra_providers::{CompletionRequest, Message, ModelProvider, ProviderError, Usage};
use ra_roadmap::model::{Node, NodeKind, TaskState};
use ra_roadmap::store::Store;
use ra_roadmap::tree;
use ra_usage::{Entry, Totals, UsageStore};

use crate::secret;

/// Where a model id routes: a bare Claude id runs on the local Claude Code; an id with a vendor
/// prefix, such as `z-ai/glm-5.3-flash`, goes through OpenRouter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    ClaudeCode,
    OpenRouter,
}

impl Route {
    /// The route of a model id.
    pub fn of(model: &str) -> Self {
        if model.contains('/') { Self::OpenRouter } else { Self::ClaudeCode }
    }
}

/// The console's handle on a worker session: the model of the badge, the provider behind it, and
/// the ledger every completion lands in. The provider connects on first use and stays for the
/// session; the clones in the background jobs share it and the session tally. A model on another
/// route reconnects, and [`Worker::forget`] drops the provider when the key changes.
#[derive(Clone)]
pub struct Worker {
    /// The model the next session runs on, as the badge holds it.
    pub model: String,
    provider: Arc<Mutex<Option<(Route, Arc<dyn ModelProvider>)>>>,
    /// The usage store; `None` keeps the tally in memory only.
    ledger: Option<UsageStore>,
    /// The session's running totals, shared by every clone.
    tally: Arc<Mutex<Totals>>,
}

impl Worker {
    /// A worker on `model`, not yet connected and without a ledger.
    pub fn new(model: impl Into<String>) -> Self {
        Self { model: model.into(), provider: Arc::new(Mutex::new(None)), ledger: None, tally: Arc::new(Mutex::new(Totals::default())) }
    }

    /// The same worker, writing every completion to `ledger`.
    pub fn with_ledger(mut self, ledger: UsageStore) -> Self {
        self.ledger = Some(ledger);
        self
    }

    /// The same worker, already connected to `provider` for its model's route; the tests hand in
    /// a scripted one.
    pub fn with_provider(self, provider: Arc<dyn ModelProvider>) -> Self {
        *self.provider.lock() = Some((self.route(), provider));
        self
    }

    /// The session's totals so far.
    pub fn tally(&self) -> Totals {
        *self.tally.lock()
    }

    /// The ledger's all-time totals; `None` without a ledger, or when it cannot be read.
    pub async fn all_time(&self) -> Option<Totals> {
        let ledger = self.ledger.as_ref()?;
        match ledger.totals().await {
            Ok(totals) => Some(totals),
            Err(e) => {
                tracing::warn!("cannot read the usage ledger: {e:#}");
                None
            }
        }
    }

    /// Books one completion against the tally and the ledger. A ledger that cannot be written is
    /// a warning, not a failed run: the answer already came back.
    async fn record(&self, node: Option<i64>, usage: &Usage) {
        let entry = Entry {
            model: self.model.clone(),
            node,
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            cached_tokens: usage.cached_tokens,
            cost_usd: usage.cost_usd,
        };
        self.tally.lock().add(&entry);
        if let Some(ledger) = &self.ledger {
            if let Err(e) = ledger.record(&entry).await {
                tracing::warn!("cannot write the usage ledger: {e:#}");
            }
        }
    }

    /// The route of the badge model.
    pub fn route(&self) -> Route {
        Route::of(&self.model)
    }

    /// The provider of the badge model, connected on the first call. OpenRouter needs the key
    /// from the credential store or the environment, and a missing one is the error; Claude Code
    /// needs only its own login.
    pub fn provider(&self) -> anyhow::Result<Arc<dyn ModelProvider>> {
        let route = self.route();
        let mut slot = self.provider.lock();
        if let Some((connected, provider)) = slot.as_ref() {
            if *connected == route {
                return Ok(provider.clone());
            }
        }
        let provider: Arc<dyn ModelProvider> = match route {
            Route::ClaudeCode => Arc::new(ra_providers::claude_code::ClaudeCode::new()),
            Route::OpenRouter => Arc::new(ra_providers::openrouter::OpenRouter::new(secret::api_key()?)?),
        };
        *slot = Some((route, provider.clone()));
        Ok(provider)
    }

    /// Drops the connected provider, so the next call connects with the current key.
    pub fn forget(&self) {
        *self.provider.lock() = None;
    }
}

/// Sends one message and returns the answer and the usage as log lines.
pub async fn ping(worker: &Worker) -> anyhow::Result<Vec<String>> {
    let provider = worker.provider()?;
    let answer = provider
        .complete(CompletionRequest {
            model: worker.model.clone(),
            messages: vec![Message::user("Reply with the single word: pong.")],
            temperature: Some(0.0),
            max_tokens: Some(16),
            tools: Vec::new(),
            workspace: None,
        })
        .await
        .map_err(map_error)?;
    worker.record(None, &answer.usage).await;
    Ok(vec![
        format!("{} says: {}", short(&worker.model), answer.text.trim()),
        format!(
            "  tokens: in {} out {} cached {} · {}",
            answer.usage.prompt_tokens,
            answer.usage.completion_tokens,
            answer.usage.cached_tokens,
            cost_text(answer.usage.cost_usd)
        ),
    ])
}

/// Runs one roadmap task: builds the context from the node, its ancestors and its dependencies,
/// sends it to the model, and writes the answer and the state back. A failed call leaves the node
/// failed, not in progress.
pub async fn run(store: &Store, id: i64, worker: &Worker) -> anyhow::Result<Vec<String>> {
    // the node first, so a bad id or a project is named before the key matters
    let node = store.read(id).await?;
    anyhow::ensure!(node.kind != NodeKind::Project, "#{} is a project; run a task", id);
    let provider = worker.provider()?;
    store.set_state(id, TaskState::InProgress).await?;

    let context = context_of(store, &node).await?;
    // the worker acts in the directory of the roadmap file: the project it describes
    let workspace =
        store.path().parent().filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf).or_else(|| std::env::current_dir().ok());
    let session = if provider.has_own_tools() {
        // a harness acts in the workspace by itself: one call carries the whole task
        let answer = provider
            .complete(CompletionRequest {
                model: worker.model.clone(),
                messages: vec![Message::system(SYSTEM), Message::user(context)],
                temperature: Some(0.4),
                max_tokens: Some(4096),
                tools: Vec::new(),
                workspace,
            })
            .await
            .map_err(map_error);
        match answer {
            Ok(answer) => {
                worker.record(Some(id), &answer.usage).await;
                Ok(Session { text: answer.text, usage: answer.usage, calls: 0, turns: 1 })
            }
            Err(error) => Err(error),
        }
    } else {
        tool_loop(provider.as_ref(), worker, id, context, workspace.as_deref()).await
    };
    let session = match session {
        Ok(session) => session,
        Err(error) => {
            store.set_state(id, TaskState::Failed).await?;
            return Err(error);
        }
    };
    store.set_notes(id, &session.text).await?;
    store.set_state(id, TaskState::Implemented).await?;
    Ok(vec![
        format!(
            "✓ #{} implemented on {} · {} turn(s), {} tool call(s) · in {} out {} · {}",
            node.title,
            short(&worker.model),
            session.turns,
            session.calls,
            session.usage.prompt_tokens,
            session.usage.completion_tokens,
            cost_text(session.usage.cost_usd)
        ),
        "  the full answer is in the notes of the card in rhipper.md".to_owned(),
    ])
}

/// What one worker session came to: the final answer, the usage summed over its turns, and how
/// much it did.
struct Session {
    text: String,
    usage: Usage,
    calls: usize,
    turns: usize,
}

/// The most turns a tool loop takes before it stops the worker.
const MAX_TURNS: usize = 40;

/// The tool loop of a bare model: the model answers with tool calls, the toolbox executes them in
/// the workspace, the results go back, until the model answers in words or the turns run out.
async fn tool_loop(
    provider: &dyn ModelProvider,
    worker: &Worker,
    id: i64,
    context: String,
    workspace: Option<&Path>,
) -> anyhow::Result<Session> {
    let workspace = workspace.context("the worker has no workspace to act in")?;
    let toolbox = ra_tools::Toolbox::new(workspace)?;
    let mut messages = vec![Message::system(SYSTEM), Message::user(context)];
    let mut usage = Usage::default();
    let mut calls = 0usize;
    for turn in 1..=MAX_TURNS {
        let answer = provider
            .complete(CompletionRequest {
                model: worker.model.clone(),
                messages: messages.clone(),
                temperature: Some(0.2),
                max_tokens: Some(4096),
                tools: ra_tools::Toolbox::specs(),
                workspace: Some(workspace.to_path_buf()),
            })
            .await
            .map_err(map_error)?;
        // every turn is a completion of its own, so each lands in the ledger as it happens
        worker.record(Some(id), &answer.usage).await;
        add_usage(&mut usage, &answer.usage);
        if answer.tool_calls.is_empty() {
            return Ok(Session { text: answer.text, usage, calls, turns: turn });
        }
        messages.push(Message::assistant_calls(answer.text, answer.tool_calls.clone()));
        for call in &answer.tool_calls {
            tracing::debug!("#{id} turn {turn}: {}({})", call.name, call.arguments);
            let result = toolbox.execute(call).await;
            messages.push(Message::tool(&call.id, result));
            calls += 1;
        }
    }
    anyhow::bail!("the worker made {calls} tool call(s) in {MAX_TURNS} turns without finishing; the task may need splitting")
}

/// Adds one completion's usage to a running sum.
fn add_usage(sum: &mut Usage, one: &Usage) {
    sum.prompt_tokens += one.prompt_tokens;
    sum.completion_tokens += one.completion_tokens;
    sum.cached_tokens += one.cached_tokens;
    if let Some(cost) = one.cost_usd {
        sum.cost_usd = Some(sum.cost_usd.unwrap_or(0.0) + cost);
    }
}

/// Reassesses one node: the model revises the objective and the acceptance for the work still
/// ahead, from the node's current text and its notes. With `recursive`, the whole subtree follows,
/// parent first.
pub async fn reassess(store: &Store, id: i64, worker: &Worker, recursive: bool) -> anyhow::Result<Vec<String>> {
    let provider = worker.provider()?;
    let nodes = store.all().await?;
    // one pass over the tree before any call, so the recursion cannot stall midway
    let order: Vec<i64> = if recursive { tree::subtree(&nodes, id).iter().map(|n| n.id).collect() } else { vec![id] };
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
            receipts.push(format!("#{} {} [{}] verified: no revision needed", node.id, node.title, node.state));
            verified.push(format!("#{}", current));
            continue;
        }
        match reassess_one(store, worker, provider.as_ref(), current, &nodes).await {
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
        short(&worker.model),
        verified.len(),
        revised.join(" ")
    )];
    out.extend(receipts);
    out.extend(warnings);
    Ok(out)
}

/// One reassessment: the model returns two lines, and the store takes them. The prompt carries the
/// node's own notes and its children with their states, so the revision names only the work the
/// subtree still owes.
async fn reassess_one(store: &Store, worker: &Worker, provider: &dyn ModelProvider, id: i64, nodes: &[Node]) -> anyhow::Result<String> {
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
            ask.push_str(&format!("- #{} {} [{}]: {}\n", kid.id, kid.title, kid.state, kid.objective.as_deref().unwrap_or("no objective")));
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
            model: worker.model.clone(),
            messages: vec![Message::system(ARCHITECT), Message::user(ask)],
            temperature: Some(0.2),
            max_tokens: Some(512),
            tools: Vec::new(),
            workspace: None,
        })
        .await
        .map_err(map_error)?;
    worker.record(Some(id), &answer.usage).await;
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
                acceptance = Some(rest.to_owned());
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

/// The system prompt of a worker session. A harness with tools implements in the workspace; a
/// bare model answers with the plan and the changes.
const SYSTEM: &str = "You are an implementation worker of a roadmap-driven orchestrator. You receive one task with its objective, acceptance criteria and related context. When you have tools, implement the task in the working directory, check it against the acceptance criteria, and answer with a short report of what you changed and what you verified. Without tools, answer with a plan of the implementation and the code or the changes it names, nothing else.";

/// The system prompt of a reassessment.
const ARCHITECT: &str = "You are the architect of a roadmap-driven orchestrator. You receive one roadmap node with its context. You keep the objective and the acceptance short, one topic per sentence, and you name the remaining work, not the work already done.";

/// The prompt of one task: the node, its ancestors, and its dependencies. One read of the store
/// serves them all.
async fn context_of(store: &Store, node: &Node) -> anyhow::Result<String> {
    let nodes = store.all().await?;
    let find = |id: i64| nodes.iter().find(|n| n.id == id);
    let mut text = format!("Task #{}: {}\n", node.id, node.title);
    if let Some(objective) = &node.objective {
        text.push_str(&format!("Objective: {objective}\n"));
    }
    if let Some(acceptance) = &node.acceptance {
        text.push_str(&format!("Acceptance criteria: {acceptance}\n"));
    }
    // the ancestors: the chain of parents above the task, nearest first
    let mut parent = node.parent.and_then(find);
    while let Some(ancestor) = parent {
        text.push_str(&format!("Under #{} {}: {}\n", ancestor.id, ancestor.title, ancestor.objective.as_deref().unwrap_or("no objective")));
        parent = ancestor.parent.and_then(find);
    }
    // the dependencies, with their own context and the notes of earlier attempts; a dangling id
    // is named, so a hand-edited file shows its mistake
    for id in &node.depends_on {
        let Some(dep) = find(*id) else {
            text.push_str(&format!("Depends on #{id}: no such node\n"));
            continue;
        };
        text.push_str(&format!(
            "Depends on #{} {} [{}]: {}\n",
            dep.id,
            dep.title,
            dep.state,
            dep.objective.as_deref().unwrap_or("no objective")
        ));
        if let Some(notes) = &dep.notes {
            text.push_str(&format!("  notes of #{}: {notes}\n", dep.id));
        }
    }
    Ok(text)
}

/// Maps a provider error to a refusal that names what to do.
fn map_error(error: ProviderError) -> anyhow::Error {
    match error {
        ProviderError::Auth(_) => anyhow::anyhow!("the provider refused the key: check OPENROUTER_API_KEY, or set one with key <paste>"),
        ProviderError::UnknownModel(model) => anyhow::anyhow!("the provider knows no model {model}"),
        ProviderError::Unavailable(text) => anyhow::anyhow!("the provider is unreachable: {text}"),
        ProviderError::Other(text) => anyhow::anyhow!("the run failed: {text}"),
    }
}

/// A cost for a log line: the dollars, or a word when the provider reported none.
fn cost_text(cost: Option<f64>) -> String {
    cost.map_or_else(|| "cost unreported".to_owned(), |c| format!("${c:.4}"))
}

/// The model id without its vendor.
fn short(model: &str) -> &str {
    model.rsplit('/').next().unwrap_or(model)
}
