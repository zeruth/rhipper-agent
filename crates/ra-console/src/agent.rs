//! The agent loop: the console's live path to a model. `ping` proves the connection; `run` sends
//! one roadmap task to the badge model and records what came back. The functions talk to a
//! [`ModelProvider`] through a [`Worker`], never to a vendor.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use parking_lot::Mutex;
use ra_providers::{CompletionRequest, Message, ModelProvider, ProviderError, Usage};
use ra_roadmap::model::{Node, NodeKind, TaskState};
use ra_roadmap::store::Store;
use ra_roadmap::tree;
use ra_usage::{Entry, Totals, UsageStore};

/// The project context file, named where the window and the console speak of it.
pub use ra_context::PROJECT_FILE;
use ra_gotchas::{Catalogue, Journal};

use crate::secret;

/// The catalogue of the gotchas, beside the roadmap file.
const GOTCHAS_FILE: &str = "var/gotchas.toml";
/// The journal of the gotchas that fired.
const GOTCHAS_LOG: &str = "var/gotchas.log";
/// How many of the latest hits the machine notes of a run draw on.
const RECENT_HITS: usize = 30;

/// Where a model id routes: a bare Claude id runs on the local Claude Code; `local/<name>` goes to
/// the local OpenAI-compatible server at `RHIPPER_LOCAL_URL`; any other id with a vendor prefix,
/// such as `z-ai/glm-5.3-flash`, goes through OpenRouter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    ClaudeCode,
    OpenRouter,
    Local,
}

impl Route {
    /// The route of a model id.
    pub fn of(model: &str) -> Self {
        if model.starts_with("local/") {
            Self::Local
        } else if model.contains('/') {
            Self::OpenRouter
        } else {
            Self::ClaudeCode
        }
    }

    /// The name of the route for a log line.
    pub fn name(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::OpenRouter => "OpenRouter",
            Self::Local => "the local server",
        }
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
    /// The dollar budget of a run, from `--budget`; `None` takes it from the node's complexity.
    pub budget: Option<f64>,
    /// Whether a run without an explicit budget takes one from the node's complexity. On unless
    /// `RHIPPER_BUDGETS=off` is in the environment, `--no-budget` was passed, or `budgets off`
    /// was typed: a subscription has no bill to guard.
    pub budgets: bool,
    /// What answers the commands of a run instead of the shell; the tests script the check and
    /// the lint with it.
    script: Option<ra_tools::Script>,
}

impl Worker {
    /// A worker on `model`, not yet connected and without a ledger.
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            provider: Arc::new(Mutex::new(None)),
            ledger: None,
            tally: Arc::new(Mutex::new(Totals::default())),
            budget: None,
            budgets: !std::env::var("RHIPPER_BUDGETS")
                .is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "off" | "0" | "false" | "no")),
            script: None,
        }
    }

    /// The same worker, with `script` answering every command a run makes instead of the shell.
    #[must_use]
    pub fn with_script(mut self, script: ra_tools::Script) -> Self {
        self.script = Some(script);
        self
    }

    /// The toolbox of a run over `root`, scripted when the worker is.
    fn toolbox(&self, root: &Path) -> anyhow::Result<ra_tools::Toolbox> {
        let toolbox = ra_tools::Toolbox::new(root)?;
        Ok(match &self.script {
            Some(script) => toolbox.with_script(script.clone()),
            None => toolbox,
        })
    }

    /// The same worker, with every run capped at `budget` dollars instead of the complexity's.
    pub fn with_budgets(mut self, on: bool) -> Self {
        self.budgets = on;
        self
    }

    /// The same worker, with every run capped at `budget` dollars instead of the complexity's.
    pub fn with_budget(mut self, budget: Option<f64>) -> Self {
        self.budget = budget;
        self
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

    /// The ledger's totals by model, the costliest first; `None` without a ledger, or when it
    /// cannot be read.
    pub async fn by_model(&self) -> Option<Vec<(String, Totals)>> {
        let ledger = self.ledger.as_ref()?;
        match ledger.by_model().await {
            Ok(rows) => Some(rows),
            Err(e) => {
                tracing::warn!("cannot read the usage ledger: {e:#}");
                None
            }
        }
    }

    /// The ledger's totals by roadmap node, `None` for the calls that served none; `None` as a
    /// whole without a ledger, or when it cannot be read.
    pub async fn by_node(&self) -> Option<Vec<(Option<i64>, Totals)>> {
        let ledger = self.ledger.as_ref()?;
        match ledger.by_node().await {
            Ok(rows) => Some(rows),
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
    /// needs only its own login; the local server needs no key, only to be running.
    pub fn provider(&self) -> anyhow::Result<Arc<dyn ModelProvider>> {
        let route = self.route();
        let mut slot = self.provider.lock();
        if let Some((connected, provider)) = slot.as_ref() {
            if *connected == route {
                return Ok(provider.clone());
            }
        }
        let provider: Arc<dyn ModelProvider> = match route {
            Route::ClaudeCode => {
                // the gotchas guard every harness run: the hooks call this very binary, and the
                // catalogue lives beside the roadmap; without either, the harness runs unguarded
                let mut harness = ra_providers::claude_code::ClaudeCode::new();
                match (std::env::current_exe(), Catalogue::load(Path::new(GOTCHAS_FILE))) {
                    (Ok(rc), Ok(catalogue)) => harness = harness.with_gotchas(rc, catalogue),
                    (_, Err(e)) => tracing::warn!("the gotchas are off: {e:#}"),
                    (Err(e), _) => tracing::warn!("the gotchas are off: no path to this binary: {e}"),
                }
                Arc::new(harness)
            }
            Route::OpenRouter => Arc::new(ra_providers::openrouter::OpenRouter::new(secret::api_key()?)?),
            // the same wire format as OpenRouter, at the local URL and with no key
            Route::Local => Arc::new(ra_providers::openrouter::OpenRouter::local()?),
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
            budget_usd: None,
            resume: None,
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
/// failed, not in progress. The run announces its start and its outcome on the progress channel,
/// so the console draws its steps in a bubble of their own.
pub async fn run(store: &Store, id: i64, worker: &Worker) -> anyhow::Result<Vec<String>> {
    // the node first, so a bad id or a project is named before the key matters
    let node = store.read(id).await?;
    anyhow::ensure!(node.kind != NodeKind::Project, "#{} is a project; run a task", id);
    tracing::info!(target: crate::trace::PROGRESS, "{}#{} {} · {}", crate::logs::RUN_START, id, node.title, short(&worker.model));
    let outcome = run_task(store, node, worker).await;
    match &outcome {
        Ok((_, cost, 0)) => tracing::info!(target: crate::trace::PROGRESS, "{}✓ {}", crate::logs::RUN_END, cost_text(*cost)),
        Ok((_, cost, gotchas)) => {
            tracing::info!(target: crate::trace::PROGRESS, "{}✓ {} · {gotchas} gotchas", crate::logs::RUN_END, cost_text(*cost));
        }
        Err(e) => tracing::info!(target: crate::trace::PROGRESS, "{}✗ {}", crate::logs::RUN_END, first_line(&format!("{e:#}"))),
    }
    outcome.map(|(lines, _, _)| lines)
}

/// The work of [`run`], with the cost of the session and the count of the gotchas it hit beside
/// the log lines.
async fn run_task(store: &Store, node: Node, worker: &Worker) -> anyhow::Result<(Vec<String>, Option<f64>, usize)> {
    let id = node.id;
    let provider = worker.provider()?;
    store.set_state(id, TaskState::InProgress).await?;

    // the worker acts in the directory of the roadmap file: the project it describes
    let workspace =
        store.path().parent().filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf).or_else(|| std::env::current_dir().ok());
    let workspace_dir = workspace.clone();
    let root = workspace_dir.clone().unwrap_or_else(|| PathBuf::from("."));
    // the context of the task: the brief of the card, the outlines of its files, and the
    // handoffs of its earlier runs, so the worker reads regions and continues rather than
    // exploring again; the project context rides along as a system message
    let mut task = ra_context::TaskContext::open(&root, id);
    task.brief = context_of(store, &node).await?;
    task.outlines = ra_context::outline_files(&root, &node.files);
    let context = task.prompt();
    let project = ra_context::Project::ensure(&root).unwrap_or_default();
    // what is dirty before the run, so the handoff names only what the run changed
    let dirty_before = git_status(&root).await;
    // an explicit budget, else the complexity's while budgets are on, else none at all
    let budget = worker.budget.or_else(|| worker.budgets.then(|| budget_for(node.complexity)));
    // the gotchas the run hits, for the journal and the summary
    let mut gotcha_hits: Vec<ra_gotchas::Hit> = Vec::new();
    let session = if provider.has_own_tools() {
        // a harness acts in the workspace by itself: one call carries the whole task, and the
        // harness holds it to the budget. The machine notes name what tripped the last runs.
        let mut messages = vec![Message::system(SYSTEM)];
        if !project.is_empty() {
            messages.push(Message::system(format!("The project, from {}:\n{project}", ra_context::PROJECT_FILE)));
        }
        let notes = machine_notes(&root);
        if !notes.is_empty() {
            messages.push(Message::system(notes));
        }
        messages.push(Message::user(context));
        let answer = provider
            .complete(CompletionRequest {
                model: worker.model.clone(),
                messages,
                temperature: Some(0.4),
                max_tokens: Some(4096),
                tools: Vec::new(),
                workspace,
                budget_usd: budget,
                // the session of the last run of this node, so the harness keeps its context
                resume: node.session.clone(),
            })
            .await
            .map_err(map_error);
        match answer {
            Ok(answer) => {
                worker.record(Some(id), &answer.usage).await;
                gotcha_hits.clone_from(&answer.gotchas);
                if let Some(session) = answer.session_id.as_deref() {
                    // kept on the node: the next run of it resumes this session
                    store.set_session(id, Some(session)).await?;
                }
                Ok(Session { text: answer.text, usage: answer.usage, calls: 0, turns: 1 })
            }
            Err(error) => Err(error),
        }
    } else {
        // a bare model reads the project context at the head of its task
        let context = if project.is_empty() { context } else { format!("The project:\n{project}\n\n{context}") };
        tool_loop(provider.as_ref(), worker, id, context, workspace.as_deref(), budget).await
    };
    // the handoff of this run, for the next one of the card; the done cards' files go
    let handoff = |outcome: &str, check: &str, report: &str, changed: Vec<String>| ra_context::Handoff {
        when: unix_now(),
        model: worker.model.clone(),
        outcome: outcome.to_owned(),
        check: check.to_owned(),
        changed,
        report: report.to_owned(),
    };
    let session = match session {
        Ok(session) => session,
        Err(error) => {
            // a spent budget is worth a note: the next attempt sees what the last one cost
            if let Some(over) = error.downcast_ref::<OverBudget>() {
                store.set_notes(id, &over.to_string()).await?;
            }
            store.set_state(id, TaskState::Failed).await?;
            let changed = ra_context::changed_between(&dirty_before, &git_status(&root).await);
            task.record(handoff("failed", "no check: the run failed", &format!("{error:#}"), changed));
            save_context(store, &root, &task).await;
            return Err(error);
        }
    };
    // a local check before the card counts as done: no model call, the acceptance names it
    let command = check_command(node.acceptance.as_deref());
    let toolbox = worker.toolbox(workspace_dir.as_deref().unwrap_or(Path::new(".")));
    let check = match &toolbox {
        Ok(toolbox) => toolbox.check(&command).await,
        Err(e) => Err(anyhow::anyhow!("{e:#}")),
    };
    let changed = ra_context::changed_between(&dirty_before, &git_status(&root).await);
    let (mut passed, mut check_line) = match check {
        Ok((true, _)) => (true, format!("  check passed: {command}")),
        Ok((false, output)) => {
            store.set_notes(id, &format!("{}\ncheck failed: {command}\n{}", session.text, last_lines(&output, 40))).await?;
            (false, format!("  check failed: {command} · the output is in the notes"))
        }
        Err(e) => {
            store.set_notes(id, &format!("{}\ncheck failed: {command}\n{e:#}", session.text)).await?;
            (false, format!("  check failed: {command} · {e:#}"))
        }
    };
    // the lint of what the run changed, once the check passed: the orchestrator finds the
    // warnings so the worker spends no turns on them. A lint that cannot run is a line, not a
    // verdict; a warning in a changed file is.
    let mut report = session.text.clone();
    let mut lint_lines = Vec::new();
    if passed && let Ok(toolbox) = &toolbox {
        let lint = lint_changed(toolbox, &root, &changed).await;
        for skipped in &lint.skipped {
            lint_lines.push(format!("  lint skipped: {skipped}"));
        }
        if lint.warnings.is_empty() {
            if !lint.crates.is_empty() {
                lint_lines.push(format!("  lint clean: cargo clippy -p {}", lint.crates.join(" -p ")));
            }
        } else {
            passed = false;
            report = format!("{}\n\n{}", lint.warnings.join("\n\n"), session.text);
            check_line = format!(
                "  check passed: {command} · lint: {} warning(s) in the changed files from cargo clippy -p {} · the lines are in the notes",
                lint.warnings.len(),
                lint.crates.join(" -p ")
            );
        }
    }
    if passed {
        store.set_notes(id, &session.text).await?;
        store.set_state(id, TaskState::Implemented).await?;
    } else {
        if report != session.text {
            store.set_notes(id, &report).await?;
        }
        store.set_state(id, TaskState::NeedsRevision).await?;
    }
    task.record(handoff(if passed { "implemented" } else { "needs revision" }, check_line.trim(), &report, changed));
    save_context(store, &root, &task).await;
    let mut lines = vec![
        format!(
            "{} #{} {} {} on {} · {} turn(s), {} tool call(s) · in {} out {} · {}",
            if passed { "✓" } else { "✗" },
            id,
            node.title,
            if passed { "implemented" } else { "needs revision" },
            short(&worker.model),
            session.turns,
            session.calls,
            session.usage.prompt_tokens,
            session.usage.completion_tokens,
            cost_text(session.usage.cost_usd)
        ),
        check_line,
    ];
    lines.extend(lint_lines);
    lines.push("  the full answer is in the notes of the card in rhipper.md".to_owned());
    if !gotcha_hits.is_empty() {
        let mut ids: Vec<&str> = gotcha_hits.iter().map(|h| h.id.as_str()).collect();
        ids.dedup();
        lines.push(format!("  ⚠ {} gotcha(s): {} · rc gotchas lists them", gotcha_hits.len(), ids.join(", ")));
    }
    Ok((lines, session.usage.cost_usd, gotcha_hits.len()))
}

/// What the lint of a run's changed files came to.
#[derive(Default)]
struct Lint {
    /// The crates linted, one `cargo clippy -p` each, in order.
    crates: Vec<String>,
    /// The diagnostics pointing at a changed file, whole.
    warnings: Vec<String>,
    /// The lints that could not run, each with its reason.
    skipped: Vec<String>,
}

/// Lints each crate that owns one of `changed` and keeps the diagnostics that point at those
/// files. Clippy exits 0 on a warning, so the output decides, not the exit code.
async fn lint_changed(toolbox: &ra_tools::Toolbox, root: &Path, changed: &[String]) -> Lint {
    let mut lint = Lint::default();
    for file in changed {
        if let Some(krate) = ra_tools::crate_of(root, file)
            && !lint.crates.contains(&krate)
        {
            lint.crates.push(krate);
        }
    }
    for krate in &lint.crates {
        let command = format!("cargo clippy -p {krate}");
        match toolbox.check(&command).await {
            Ok((_, output)) => lint.warnings.extend(ra_tools::warnings_at(&output, changed)),
            Err(e) => lint.skipped.push(format!("{command} · {e:#}")),
        }
    }
    lint
}

/// Writes a task's context file and removes the files of the cards that are done. A failure
/// here is a log line, not a failed run: the context is a saving, not the work.
async fn save_context(store: &Store, root: &Path, task: &ra_context::TaskContext) {
    if let Err(e) = task.save(root) {
        tracing::warn!("the context of #{} was not saved: {e:#}", task.id);
    }
    if let Ok(nodes) = store.all().await {
        let open: Vec<i64> =
            nodes.iter().filter(|n| !matches!(n.state, TaskState::Accepted | TaskState::Cancelled)).map(|n| n.id).collect();
        ra_context::TaskContext::prune(root, &open);
    }
}

/// The context a run of `id` would send, without running: the brief, the outlines and the
/// handoffs so far. For `rc context <id>` and the window.
pub async fn context_preview(store: &Store, id: i64) -> anyhow::Result<String> {
    let node = store.read(id).await?;
    let root = workspace_of(store);
    let mut task = ra_context::TaskContext::open(&root, id);
    task.brief = context_of(store, &node).await?;
    task.outlines = ra_context::outline_files(&root, &node.files);
    Ok(task.prompt())
}

/// The project context a run sends ahead of the task, as the file beside the roadmap holds
/// it; empty while there is none. Reads only: a run drafts the file, a preview does not.
#[must_use]
pub fn project_context(store: &Store) -> String {
    ra_context::Project::load(&workspace_of(store)).unwrap_or_default()
}

/// The workspace: the directory of the roadmap file.
fn workspace_of(store: &Store) -> PathBuf {
    store.path().parent().filter(|p| !p.as_os_str().is_empty()).map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// `git status --porcelain` of `root`, or nothing where git has nothing to say.
async fn git_status(root: &Path) -> String {
    match tokio::process::Command::new("git").args(["status", "--porcelain"]).current_dir(root).output().await {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).into_owned(),
        _ => String::new(),
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The check of a node: the first backticked command in its acceptance that starts with cargo,
/// else the tests of the whole workspace.
pub fn check_command(acceptance: Option<&str>) -> String {
    acceptance
        .and_then(|text| {
            text.split('`').skip(1).step_by(2).map(str::trim).find(|c| c == &"cargo" || c.starts_with("cargo ")).map(str::to_owned)
        })
        .unwrap_or_else(|| "cargo test --workspace -q".to_owned())
}

/// The last `n` lines of a text.
fn last_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// The machine notes of a run: what the gotchas say about the last hits in the journal, for the
/// system prompt, so the worker does not find out the hard way again. Empty when nothing fired.
fn machine_notes(workspace: &Path) -> String {
    let Ok(catalogue) = Catalogue::load(&workspace.join(GOTCHAS_FILE)) else { return String::new() };
    catalogue.notes(&Journal::at(&workspace.join(GOTCHAS_LOG)).recent(RECENT_HITS))
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

/// The dollar budget of a run on a node of `complexity`, when `--budget` names none: c1 $0.25,
/// c2 $0.50, c3 $1.00, c4 $2.00, c5 $5.00. An out-of-range complexity takes the nearest end.
pub fn budget_for(complexity: u8) -> f64 {
    match complexity {
        0 | 1 => 0.25,
        2 => 0.50,
        3 => 1.00,
        4 => 2.00,
        _ => 5.00,
    }
}

/// A run stopped because its summed cost crossed its budget. Its text is the note on the node.
#[derive(Debug)]
struct OverBudget {
    budget: f64,
    spent: f64,
    turns: usize,
    calls: usize,
}

impl std::fmt::Display for OverBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "stopped over budget: spent ${:.4} of a ${:.2} budget in {} turn(s), {} tool call(s); raise it with --budget or split the task",
            self.spent, self.budget, self.turns, self.calls
        )
    }
}

impl std::error::Error for OverBudget {}

/// The tool loop of a bare model: the model answers with tool calls, the toolbox executes them in
/// the workspace, the results go back, until the model answers in words, the turns run out, or
/// the summed cost crosses `budget`.
async fn tool_loop(
    provider: &dyn ModelProvider,
    worker: &Worker,
    id: i64,
    context: String,
    workspace: Option<&Path>,
    budget: Option<f64>,
) -> anyhow::Result<Session> {
    let workspace = workspace.context("the worker has no workspace to act in")?;
    let toolbox = worker.toolbox(workspace)?;
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
                budget_usd: budget,
                resume: None,
            })
            .await
            .map_err(map_error)?;
        // every turn is a completion of its own, so each lands in the ledger as it happens
        worker.record(Some(id), &answer.usage).await;
        add_usage(&mut usage, &answer.usage);
        if answer.tool_calls.is_empty() {
            return Ok(Session { text: answer.text, usage, calls, turns: turn });
        }
        // the budget is checked before the tools run: the work it would pay for is not done
        if let Some((budget, spent)) = budget.zip(usage.cost_usd).filter(|(budget, spent)| spent > budget) {
            return Err(OverBudget { budget, spent, turns: turn, calls }.into());
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
///
/// A node remembers what its last reassessment saw in its `assessed:` line. A node marked final,
/// or one whose fingerprint still matches, is assessed and unchanged: no model call.
pub async fn reassess(store: &Store, id: i64, worker: &Worker, recursive: bool) -> anyhow::Result<Vec<String>> {
    let nodes = store.all().await?;
    // one pass over the tree before any call, so the recursion cannot stall midway
    let order: Vec<i64> = if recursive { tree::subtree(&nodes, id).iter().map(|n| n.id).collect() } else { vec![id] };
    // the provider connects on the first node that needs a call, so an unchanged tree costs nothing
    let mut provider: Option<Arc<dyn ModelProvider>> = None;
    let mut revised: Vec<String> = Vec::new();
    let mut verified: Vec<String> = Vec::new();
    let mut unchanged: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut receipts: Vec<String> = Vec::new();
    for current in order {
        // a fresh read per node: a revision earlier in the pass changes the inputs of its kin
        let nodes = store.all().await?;
        let Some(node) = nodes.iter().find(|n| n.id == current) else {
            warnings.push(format!("⚠ #{}: no such node", current));
            continue;
        };
        if node.is_final() {
            receipts.push(format!("#{} {} [final] assessed and unchanged: no call", node.id, node.title));
            unchanged.push(format!("#{}", current));
            continue;
        }
        // a node whose work is done is verified, not recreated: no model call, no chance to drift
        if matches!(node.state, TaskState::Implemented | TaskState::Accepted) {
            receipts.push(format!("#{} {} [{}] verified: no revision needed", node.id, node.title, node.state));
            verified.push(format!("#{}", current));
            continue;
        }
        if node.assessed.as_deref() == Some(fingerprint(node, &nodes).as_str()) {
            receipts.push(format!("#{} {} assessed and unchanged: no call", node.id, node.title));
            unchanged.push(format!("#{}", current));
            continue;
        }
        let connected = match provider.clone() {
            Some(connected) => connected,
            None => {
                let connected = worker.provider()?;
                provider = Some(connected.clone());
                connected
            }
        };
        match reassess_one(store, worker, connected.as_ref(), current, &nodes).await {
            Ok(receipt) => {
                revised.push(format!("#{}", current));
                receipts.push(receipt);
            }
            Err(e) => warnings.push(format!("⚠ #{}: {e:#}", current)),
        }
    }
    let mut out = vec![format!(
        "✓ {} node(s) reassessed on {} ({} verified, {} unchanged): {}",
        revised.len() + verified.len() + unchanged.len(),
        short(&worker.model),
        verified.len(),
        unchanged.len(),
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
            budget_usd: None,
            resume: None,
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
    // the fingerprint of the text as it now stands, changed or not, so the next pass skips it
    let after = store.all().await?;
    if let Some(revised) = after.iter().find(|n| n.id == id) {
        store.set_assessed(id, Some(&fingerprint(revised, &after))).await?;
    }

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

/// The fingerprint of what a reassessment of `node` reads: its objective, its acceptance, the
/// first line of its notes, and each child's id, state and objective. Sixteen hex digits of a
/// 64-bit FNV-1a hash, stable across builds, so a file written today matches tomorrow.
pub fn fingerprint(node: &Node, nodes: &[Node]) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0100_0000_01b3;
    let mut hash = OFFSET;
    let mut feed = |text: &str| {
        // a unit separator after each field, so moving text across fields changes the hash
        for byte in text.bytes().chain(std::iter::once(0x1f)) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
    };
    feed(node.objective.as_deref().unwrap_or(""));
    feed(node.acceptance.as_deref().unwrap_or(""));
    feed(&node.notes.as_deref().map(first_line).unwrap_or_default());
    for kid in nodes.iter().filter(|n| n.parent == Some(node.id)) {
        feed(&kid.id.to_string());
        feed(kid.state.serde_name());
        feed(kid.objective.as_deref().unwrap_or(""));
    }
    format!("{hash:016x}")
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
const SYSTEM: &str = "You are an implementation worker of a roadmap-driven orchestrator. You receive one task with its objective, acceptance criteria and related context. When you have tools, implement the task in the working directory, check it against the acceptance criteria, and answer with a short report of what you changed and what you verified. Bash and PowerShell both run cargo, the read-only git commands and the read-only listing commands; pipe a long output through tail. After your report the orchestrator runs the check and lints the files you changed, and any warning comes back to you; do not hunt warnings yourself. Without tools, answer with a plan of the implementation and the code or the changes it names, nothing else.";

/// The system prompt of a reassessment.
const ARCHITECT: &str = "You are the architect of a roadmap-driven orchestrator. You receive one roadmap node with its context. You keep the objective and the acceptance short, one topic per sentence, and you name the remaining work, not the work already done. When the objective and the acceptance already name only the remaining work and are checkable, you answer with the same text rather than rewording it.";

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
    // the files the task names: where the change lands; the task context outlines them
    if !node.files.is_empty() {
        text.push_str("Files of the card, where the change lands:\n");
        for file in &node.files {
            text.push_str(&format!("  {file}\n"));
        }
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
