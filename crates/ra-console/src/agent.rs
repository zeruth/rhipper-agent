//! The agent loop: the console's live path to a model. `ping` proves the connection; `run` sends
//! one roadmap task to the badge model and records what came back. The functions talk to a
//! [`ModelProvider`] through a [`Worker`], never to a vendor.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use parking_lot::Mutex;
use ra_providers::{CompletionRequest, Message, ModelProvider, ProviderError, TurnUsage, Usage};
use ra_roadmap::model::{Effort, Estimate, Node, NodeKind, TaskState};
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
    /// Off unless `RHIPPER_BUDGETS=on` is in the environment or `budgets on` was typed: nothing
    /// caps a run by default, and a subscription has no bill to guard. `--budget` caps one run
    /// either way.
    pub budgets: bool,
    /// The turn cap of a run, from `--turns`; `None` takes it from the node's complexity. It
    /// holds whether budgets are on or off: on a subscription the turns are the cost.
    pub turns: Option<u32>,
    /// The effort of a run, from `--effort`; `None` takes the card's own, medium when it names
    /// none.
    pub effort: Option<Effort>,
    /// Debug mode, on unless `RHIPPER_DEBUG=off`: a run asks the worker to end its report with
    /// what would have made it faster, keeps the answer, and hands the latest to the next runs.
    pub debug: bool,
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
            budgets: std::env::var("RHIPPER_BUDGETS")
                .is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "on" | "1" | "true" | "yes")),
            turns: None,
            effort: None,
            debug: !std::env::var("RHIPPER_DEBUG")
                .is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "off" | "0" | "false" | "no")),
            script: None,
        }
    }

    /// The same worker, in or out of debug mode.
    #[must_use]
    pub fn with_debug(mut self, on: bool) -> Self {
        self.debug = on;
        self
    }

    /// The same worker, with every run capped at `turns` turns instead of the complexity's.
    #[must_use]
    pub fn with_turns(mut self, turns: Option<u32>) -> Self {
        self.turns = turns;
        self
    }

    /// The same worker, with every run at `effort` instead of the card's.
    #[must_use]
    pub fn with_effort(mut self, effort: Option<Effort>) -> Self {
        self.effort = effort;
        self
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

    /// The ledger's calls of one node, the latest first (`None` for the calls that served none);
    /// `None` as a whole without a ledger, or when it cannot be read.
    pub async fn calls_of(&self, node: Option<i64>) -> Option<Vec<ra_usage::Call>> {
        let ledger = self.ledger.as_ref()?;
        match ledger.calls_of(node).await {
            Ok(rows) => Some(rows),
            Err(e) => {
                tracing::warn!("cannot read the usage ledger: {e:#}");
                None
            }
        }
    }

    /// Books one completion of `turns` turns against the tally and the ledger. A ledger that
    /// cannot be written is a warning, not a failed run: the answer already came back.
    pub(crate) async fn record(&self, node: Option<i64>, usage: &Usage, turns: u64) {
        let entry = Entry {
            model: self.model.clone(),
            node,
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            cached_tokens: usage.cached_tokens,
            cost_usd: usage.cost_usd,
            turns,
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
                // the project's catalogue when it has one; the seed otherwise, written by the
                // hook in the run's own directory, not here (a test runs where no var/ belongs)
                let catalogue =
                    if Path::new(GOTCHAS_FILE).exists() { Catalogue::load(Path::new(GOTCHAS_FILE)) } else { Ok(Catalogue::seed()) };
                match (std::env::current_exe(), catalogue) {
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
            max_turns: None,
            effort: None,
        })
        .await
        .map_err(map_error)?;
    worker.record(None, &answer.usage, answer.turns).await;
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

/// Runs the first ready card, the one `rc next` names; a line says so when none is.
///
/// # Errors
/// When the roadmap cannot be read, or the run fails.
pub async fn run_next(store: &Store, worker: &Worker) -> anyhow::Result<Vec<String>> {
    let nodes = store.all().await?;
    match tree::next_ready(&nodes) {
        Some(node) => run(store, node.id, worker).await,
        None => Ok(vec!["no card is ready".to_owned()]),
    }
}

/// Runs the ready cards one after another, reading readiness again after each, until none is
/// ready or a run fails; with `stop_on_revision` a card that needs revision stops the batch too.
/// Each run is its own bubble and its own handoff, through [`run`]. A failed run ends the batch
/// with its reason on the last line rather than as an error, so the runs before it keep their
/// lines.
///
/// # Errors
/// When the roadmap cannot be read between runs.
pub async fn run_all(store: &Store, worker: &Worker, stop_on_revision: bool) -> anyhow::Result<Vec<String>> {
    let mut lines = Vec::new();
    let mut ran = 0;
    loop {
        let nodes = store.all().await?;
        let Some(id) = batch_ready(&nodes).map(|n| n.id) else {
            lines.push(format!("ran {ran} card(s) · no card is ready"));
            return Ok(lines);
        };
        match run(store, id, worker).await {
            Ok(run_lines) => lines.extend(run_lines),
            Err(error) => {
                lines.push(format!("ran {ran} card(s) · stopped: the run of #{id} failed: {error:#}"));
                return Ok(lines);
            }
        }
        ran += 1;
        if stop_on_revision && store.read(id).await?.state == TaskState::NeedsRevision {
            lines.push(format!("ran {ran} card(s) · stopped: #{id} needs revision"));
            return Ok(lines);
        }
    }
}

/// The first card a batch runs, in tree order: a defined task whose waits-on are each accepted or
/// implemented. Implemented counts beside accepted here, unlike in [`tree::next_ready`]: the cards
/// the batch implements are not accepted until the person reviews, and their dependents are what
/// the batch is meant to run ahead of that review. A card whose waits-on failed, left the roadmap
/// or are still open is not picked, so every pick moves a card out of defined and the loop ends.
fn batch_ready(nodes: &[Node]) -> Option<&Node> {
    let met = |dep: &i64| nodes.iter().any(|d| d.id == *dep && matches!(d.state, TaskState::Accepted | TaskState::Implemented));
    nodes
        .iter()
        .find(|n| n.state == TaskState::Defined && matches!(n.kind, NodeKind::Task | NodeKind::Subtask) && n.depends_on.iter().all(met))
}

/// The work of [`run`], with the cost of the session and the count of the gotchas it hit beside
/// the log lines.
async fn run_task(store: &Store, node: Node, worker: &Worker) -> anyhow::Result<(Vec<String>, Option<f64>, usize)> {
    let id = node.id;
    let provider = worker.provider()?;
    store.set_state(id, TaskState::InProgress).await?;

    let Prepared { workspace, root, mut task, project, budget, cap, effort } = prepare(store, &node, worker, true).await?;
    let workspace_dir = workspace.clone();
    // what is dirty before the run, so the handoff names only what the run changed
    let dirty_before = git_status(&root).await;
    // the gotchas the run hits, for the journal and the summary
    let mut gotcha_hits: Vec<ra_gotchas::Hit> = Vec::new();
    // the session of the last run of this node, so the harness keeps its context, unless it is
    // stale: then the brief and the handoffs carry it, and the run says so first
    let (resume, fresh) = match resume_of(&node, unix_now()) {
        Ok(resume) => (resume, None),
        Err(reason) => (None, Some(format!("  fresh start: {reason}; the brief and the handoffs go instead"))),
    };
    if let Some(fresh) = &fresh {
        tracing::info!(target: crate::trace::PROGRESS, "{}", fresh.trim_start());
    }
    let session = if provider.has_own_tools() {
        // a harness acts in the workspace by itself: one call carries the whole task, and the
        // harness holds it to the budget
        let answer = provider
            .complete(CompletionRequest {
                model: worker.model.clone(),
                messages: harness_messages(&task, &project, &root, worker.debug),
                temperature: Some(0.4),
                max_tokens: Some(4096),
                tools: Vec::new(),
                workspace,
                budget_usd: budget,
                resume,
                max_turns: cap,
                effort: Some(effort.to_string()),
            })
            .await
            .map_err(map_error);
        match answer {
            Ok(answer) => {
                worker.record(Some(id), &answer.usage, answer.turns).await;
                gotcha_hits.clone_from(&answer.gotchas);
                if let Some(session) = answer.session_id.as_deref() {
                    // kept on the node with the card's fingerprint: the next run of it resumes
                    // this session while the card reads the same
                    store.set_session(id, Some((session, &card_fingerprint(&node))), unix_now()).await?;
                }
                Ok(Session { text: answer.text, usage: answer.usage, calls: 0, turns: answer.turns, turn_usage: answer.turn_usage, cap })
            }
            Err(error) => Err(error),
        }
    } else {
        tool_loop(provider.as_ref(), worker, id, bare_context(&task, &project, worker.debug), workspace.as_deref(), budget, cap).await
    };
    // the handoff of this run, for the next one of the card; the done cards' files go
    let handoff = |outcome: &str, check: &str, report: &str, changed: Vec<String>| ra_context::Handoff {
        when: unix_now(),
        model: worker.model.clone(),
        outcome: outcome.to_owned(),
        check: check.to_owned(),
        changed,
        report: report.to_owned(),
        warnings: Vec::new(),
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
    // a failed check leads the notes and the handoff: the failed tests first, then the tail of
    // the output, then the worker's answer, so the next run reads the failures before anything
    let mut report = session.text.clone();
    let (mut passed, mut check_line) = match check {
        Ok((true, _)) => (true, format!("  check passed: {command}")),
        Ok((false, output)) => {
            let failure = check_failure(&output);
            report = format!("check failed: {command}\n{}\n\n{}", failure.text, session.text);
            let count = match failure.failed {
                0 => String::new(),
                n => format!(" · {n} test(s) failed"),
            };
            (false, format!("  check failed: {command}{count} · the output is in the notes"))
        }
        Err(e) => {
            report = format!("check failed: {command}\n{e:#}\n\n{}", session.text);
            (false, format!("  check failed: {command} · {e:#}"))
        }
    };
    // the lint of what the run changed, once the check passed: the orchestrator finds the
    // warnings so the worker spends no turns on them. A lint that cannot run is a line, not a
    // verdict; a warning in a changed file is.
    let mut lint_lines = Vec::new();
    let mut lint_warnings = Vec::new();
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
            lint_warnings = lint.warnings;
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
    // the run against its estimate, on the check line so the handoff carries it: the next run
    // reads what the last one spent against what the card was sized at
    let verdict = estimate_verdict(estimate_of(&node), &session.usage, session.turns);
    check_line = format!("{check_line} · {verdict}");
    // the warnings ride in their own section of the handoff, not at the head of its report: the
    // notes lead with them, the next run gets them in its machine notes
    let mut last = handoff(
        if passed { "implemented" } else { "needs revision" },
        check_line.trim(),
        if lint_warnings.is_empty() { &report } else { &session.text },
        changed,
    );
    last.warnings = lint_warnings;
    task.record(last);
    save_context(store, &root, &task).await;
    let turns = match session.cap {
        Some(cap) => format!("{}/{cap}", session.turns),
        None => session.turns.to_string(),
    };
    let mut lines = vec![
        format!(
            "{} #{} {} {} on {} · turns {turns}, {} tool call(s) · in {} out {}{} · {} · {verdict}",
            if passed { "✓" } else { "✗" },
            id,
            node.title,
            if passed { "implemented" } else { "needs revision" },
            short(&worker.model),
            session.calls,
            session.usage.prompt_tokens,
            session.usage.completion_tokens,
            turn_sizes(&session.turn_usage),
            cost_text(session.usage.cost_usd)
        ),
        check_line,
    ];
    lines.extend(fresh);
    lines.extend(lint_lines);
    // in debug mode the worker's own look back: kept for the next runs, and named here
    if worker.debug {
        let notes = crate::retro::parse(&session.text);
        if let Err(e) = crate::retro::record(&root, Some(id), &notes) {
            tracing::warn!("the look back of #{id} was not kept: {e:#}");
        }
        for note in &notes {
            lines.push(format!("  ↯ {}{}", if note.machine { "[machine] " } else { "" }, note.text));
        }
    }
    lines.push("  the full answer is in the notes of the card in rhipper.md".to_owned());
    if !gotcha_hits.is_empty() {
        let mut ids: Vec<&str> = gotcha_hits.iter().map(|h| h.id.as_str()).collect();
        ids.dedup();
        lines.push(format!("  ⚠ {} gotcha(s): {} · rc gotchas lists them", gotcha_hits.len(), ids.join(", ")));
    }
    Ok((lines, session.usage.cost_usd, gotcha_hits.len()))
}

/// What a run of a node has in hand before it calls the model: the workspace, the task context,
/// the project context, the budget and the turn cap. A run sends it; a dry run prints it.
struct Prepared {
    /// The directory the worker acts in; `None` when neither the roadmap nor the process names one.
    workspace: Option<PathBuf>,
    /// The workspace, or `.` without one: where the context files and the outlines are read.
    root: PathBuf,
    task: ra_context::TaskContext,
    /// The project context; empty while there is none.
    project: String,
    budget: Option<f64>,
    /// The turn cap, when `--turns` named one; nothing caps a run by default.
    cap: Option<u32>,
    /// The effort the run asks of the model: `--effort` when it was passed, else the card's.
    effort: Effort,
}

/// Builds what a run of `node` sends. The context of the task is the brief of the card, the
/// outlines of its files, and the handoffs of its earlier runs, so the worker reads regions and
/// continues rather than exploring again. With `draft`, a missing project context file is
/// written first, as a run does; a dry run reads what is there.
async fn prepare(store: &Store, node: &Node, worker: &Worker, draft: bool) -> anyhow::Result<Prepared> {
    // the worker acts in the directory of the roadmap file: the project it describes
    let workspace =
        store.path().parent().filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf).or_else(|| std::env::current_dir().ok());
    let root = workspace.clone().unwrap_or_else(|| PathBuf::from("."));
    let mut task = ra_context::TaskContext::open(&root, node.id);
    task.brief = context_of(store, node).await?;
    task.outlines = ra_context::outline_files(&root, &node.files, &last_change(&root, node, &task).await);
    let project =
        if draft { ra_context::Project::ensure(&root).unwrap_or_default() } else { ra_context::Project::load(&root).unwrap_or_default() };
    // an explicit budget, else the complexity's while budgets are on, else none at all; a turn
    // cap only when one was asked for. No limit stands by default: the person asked for none
    let budget = worker.budget.or_else(|| worker.budgets.then(|| budget_for(node.complexity)));
    let cap = worker.turns;
    let effort = worker.effort.unwrap_or(node.effort);
    Ok(Prepared { workspace, root, task, project, budget, cap, effort })
}

/// The messages of a harness run: the worker's role, the project context and the machine notes
/// as system messages, the task as the user turn. The machine notes name what tripped the last
/// runs, so the worker does not find out the hard way again, and the warnings the last run of
/// the card left, so a second attempt opens on the exact lines.
fn harness_messages(task: &ra_context::TaskContext, project: &str, root: &Path, debug: bool) -> Vec<Message> {
    let mut messages = vec![Message::system(SYSTEM)];
    if !project.is_empty() {
        messages.push(Message::system(format!("The project, from {}:\n{project}", ra_context::PROJECT_FILE)));
    }
    let notes = [machine_notes(root), warnings_note(task)].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n\n");
    if !notes.is_empty() {
        messages.push(Message::system(notes));
    }
    if debug {
        messages.push(Message::system(crate::retro::PROMPT));
    }
    messages.push(Message::user(task.prompt()));
    messages
}

/// The task of a bare model, which reads the project context at the head of it and, in debug
/// mode, the ask for its look back at the end.
fn bare_context(task: &ra_context::TaskContext, project: &str, debug: bool) -> String {
    let context = task.prompt();
    let mut text = if project.is_empty() { context } else { format!("The project:\n{project}\n\n{context}") };
    if debug {
        text.push_str("\n\n");
        text.push_str(crate::retro::PROMPT);
    }
    text
}

/// Everything a run of `id` would send, printed instead of sent: the system messages, the task
/// context, and the command line of the harness or the tools of a bare model. No model is
/// called, and the card keeps its state, its session and its notes. For `rc run --dry` and the
/// console's `run <id> dry`.
///
/// # Errors
/// When the card is not there or is the project, or the route has no provider.
pub async fn dry_run(store: &Store, id: i64, worker: &Worker) -> anyhow::Result<Vec<String>> {
    let node = store.read(id).await?;
    anyhow::ensure!(node.kind != NodeKind::Project, "#{id} is a project; run a task");
    let provider = worker.provider()?;
    let Prepared { workspace, root, task, project, budget, cap, effort } = prepare(store, &node, worker, false).await?;
    let harness = provider.has_own_tools();
    let messages = if harness {
        harness_messages(&task, &project, &root, worker.debug)
    } else {
        vec![Message::system(SYSTEM), Message::user(bare_context(&task, &project, worker.debug))]
    };
    let request = CompletionRequest {
        model: worker.model.clone(),
        messages,
        temperature: Some(if harness { 0.4 } else { 0.2 }),
        max_tokens: Some(4096),
        tools: if harness { Vec::new() } else { ra_tools::Toolbox::specs() },
        workspace,
        budget_usd: budget,
        resume: resume_of(&node, unix_now()).unwrap_or_default(),
        max_turns: cap,
        effort: Some(effort.to_string()),
    };
    let mut lines = vec![
        format!(
            "dry run of #{id} {} on {} via {}: nothing is called, the card is as it was",
            node.title,
            worker.model,
            worker.route().name()
        ),
        format!(
            "  budget {} · turn cap {} · resume {} · check: {}",
            budget.map_or_else(|| "none".to_owned(), |b| format!("${b:.2}")),
            cap.map_or_else(|| "none".to_owned(), |c| c.to_string()),
            match resume_of(&node, unix_now()) {
                Ok(Some(session)) => session,
                Ok(None) => "none".to_owned(),
                Err(reason) => format!("none, {reason}"),
            },
            check_command(node.acceptance.as_deref())
        ),
    ];
    let command = provider.command_line(&request);
    if let Some(command) = &command {
        // the effort and the allowed tools, named before the argument list buries them
        let effort = argument(command, "--effort").unwrap_or("the harness default");
        let allowed = arguments(command, "--allowedTools");
        lines
            .push(format!("  effort {effort} · allowed tools: {}", if allowed.is_empty() { "none".to_owned() } else { allowed.join(" ") }));
    } else {
        let names: Vec<String> = request.tools.iter().map(|t| t.name.clone()).collect();
        lines.push(format!("  effort: none, a bare model · tools: {} · temperature 0.2", names.join(" ")));
    }
    for message in &request.messages {
        lines.push(format!("--- {} ---", message.role));
        lines.extend(message.content.lines().map(str::to_owned));
    }
    if let Some(command) = command {
        lines.push("--- command ---".to_owned());
        let mut previous = "";
        for arg in &command {
            // the system prompt is the messages above; the line names its size, not its text
            lines.push(if previous == "--system-prompt" {
                format!("  <the system messages, {} chars>", arg.len())
            } else {
                format!("  {arg}")
            });
            previous = arg;
        }
        lines.push("  (the task goes on stdin)".to_owned());
    }
    Ok(lines)
}

/// The value after `flag` in a command line.
fn argument<'a>(command: &'a [String], flag: &str) -> Option<&'a str> {
    command.iter().position(|a| a == flag).and_then(|at| command.get(at + 1)).map(String::as_str)
}

/// The values after `flag` in a command line, up to the next flag.
fn arguments<'a>(command: &'a [String], flag: &str) -> Vec<&'a str> {
    match command.iter().position(|a| a == flag) {
        Some(at) => command[at + 1..].iter().map(String::as_str).take_while(|a| !a.starts_with("--")).collect(),
        None => Vec::new(),
    }
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

/// Lints each crate that owns one of `changed` and keeps the diagnostics that point into the
/// lines the change touched (`git diff -U0` of those files; a file git does not know is new,
/// and all of it counts), not the warnings its files had before. Clippy exits 0 on a warning,
/// so the output decides, not the exit code.
async fn lint_changed(toolbox: &ra_tools::Toolbox, root: &Path, changed: &[String]) -> Lint {
    let mut lint = Lint::default();
    for file in changed {
        if let Some(krate) = ra_tools::crate_of(root, file)
            && !lint.crates.contains(&krate)
        {
            lint.crates.push(krate);
        }
    }
    if lint.crates.is_empty() {
        return lint;
    }
    let diff = match toolbox.check(&format!("git diff -U0 HEAD -- {}", changed.join(" "))).await {
        Ok((_, output)) => output,
        Err(e) => {
            lint.skipped.push(format!("git diff · {e:#}"));
            String::new()
        }
    };
    let mut ranges = ra_tools::changed_ranges(&diff);
    for file in changed {
        let known = ranges.iter().any(|(f, _, _)| f == &file.replace('\\', "/"));
        if !known {
            ranges.push((file.replace('\\', "/"), 1, u32::MAX - 2));
        }
    }
    for krate in &lint.crates {
        let command = format!("cargo clippy -p {krate}");
        match toolbox.check(&command).await {
            Ok((_, output)) => lint.warnings.extend(ra_tools::warnings_within(&output, &ranges)),
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
    task.outlines = ra_context::outline_files(&root, &node.files, &last_change(&root, &node, &task).await);
    Ok(task.prompt())
}

/// The lines the last run of a card in revision changed, from `git diff -U0` over the files
/// its handoff names, so the outline marks them; nothing for any other card, whose worker
/// starts fresh.
async fn last_change(root: &Path, node: &Node, task: &ra_context::TaskContext) -> ra_context::ChangedLines {
    let files = match task.handoffs.last() {
        Some(last) if node.state == TaskState::NeedsRevision && !last.changed.is_empty() => &last.changed,
        _ => return ra_context::ChangedLines::new(),
    };
    let mut command = tokio::process::Command::new("git");
    command.args(["diff", "-U0", "HEAD", "--"]).args(files).current_dir(root);
    match command.output().await {
        Ok(out) if out.status.success() => ra_context::changed_lines(&String::from_utf8_lossy(&out.stdout)),
        _ => ra_context::ChangedLines::new(),
    }
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

pub(crate) fn unix_now() -> u64 {
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

/// How many lines of a failed check's output the notes keep after the failures.
const TAIL_LINES: usize = 40;

/// What a failed check's output comes to for the notes.
struct CheckFailure {
    /// The tests cargo marked `FAILED`; zero where the output is not a test run's.
    failed: usize,
    /// The `test … FAILED` and `panicked at` lines first, then the last [`TAIL_LINES`] lines.
    text: String,
}

/// Reads a failed check's output: the failed tests and the panics lead, the tail follows, so a
/// reader (the next run, the person) sees what broke before the noise around it. A failed test
/// is `test name ... FAILED`, or `name --- FAILED` under `-q`; cargo names it again in its
/// `failures:` list, and the first names are kept, once.
fn check_failure(output: &str) -> CheckFailure {
    let mut leading: Vec<&str> = Vec::new();
    let mut failed = 0;
    for line in output.lines().map(str::trim) {
        let is_failure = line.ends_with(" FAILED") && (line.starts_with("test ") || line.contains(" --- "));
        if (is_failure || line.contains("panicked at")) && !leading.contains(&line) {
            leading.push(line);
            failed += usize::from(is_failure);
        }
    }
    let tail = last_lines(output, TAIL_LINES);
    let text = if leading.is_empty() { tail } else { format!("{}\n\n{tail}", leading.join("\n")) };
    CheckFailure { failed, text }
}

/// The warnings the last run of the card left, as a machine note: the lint found them in the
/// lines that run changed, and they are what sent the card back. Empty when it left none.
fn warnings_note(task: &ra_context::TaskContext) -> String {
    let warnings = task.last_warnings();
    if warnings.is_empty() {
        return String::new();
    }
    format!(
        "The lint of the last run of this card left {} warning(s) in the lines it changed; they sent the card back. Fix these lines first:\n\n{}",
        warnings.len(),
        warnings.join("\n\n")
    )
}

/// The machine notes of a run: what the gotchas say about the last hits in the journal, for the
/// system prompt, so the worker does not find out the hard way again. Empty when nothing fired.
fn machine_notes(workspace: &Path) -> String {
    let gotchas = match Catalogue::load(&workspace.join(GOTCHAS_FILE)) {
        Ok(catalogue) => catalogue.notes(&Journal::at(&workspace.join(GOTCHAS_LOG)).recent(RECENT_HITS)),
        Err(_) => String::new(),
    };
    // what the earlier runs said themselves, in debug mode, rides along
    let retro = crate::retro::machine_lines(workspace);
    [gotchas, retro].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n\n")
}

/// What one worker session came to: the final answer, the usage summed over its turns, and how
/// much it did of the turns it had.
struct Session {
    text: String,
    usage: Usage,
    calls: usize,
    turns: u64,
    /// The tokens of each turn, when the provider showed them.
    turn_usage: Vec<TurnUsage>,
    /// The turn cap the run had, when it had one.
    cap: Option<u32>,
}

/// The sizes of the turns for the run line: the largest and the average, in tokens read and
/// written, as ` · per turn largest 41k, avg 12k`; nothing when the provider showed no turns.
fn turn_sizes(turns: &[TurnUsage]) -> String {
    let Ok(count) = u64::try_from(turns.len()) else { return String::new() };
    if count == 0 {
        return String::new();
    }
    let largest = turns.iter().map(TurnUsage::total).max().unwrap_or(0);
    let average = turns.iter().map(TurnUsage::total).sum::<u64>() / count;
    format!(" · per turn largest {}, avg {}", kilo(largest), kilo(average))
}

/// A token count in short: `812`, `4.1k`, `120k`.
fn kilo(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1000..=9999 => format!("{}.{}k", n / 1000, (n % 1000) / 100),
        _ => format!("{}k", n / 1000),
    }
}

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

/// The estimate of a run on a node of `complexity`, when the card names none: what a lean run
/// reads and writes over its turns. c1 20k tokens in 4 turns, c2 50k in 8, c3 120k in 15, c4
/// 250k in 25, c5 500k in 40. An out-of-range complexity takes the nearest end.
#[must_use]
pub fn estimate_for(complexity: u8) -> Estimate {
    let (tokens, turns) = match complexity {
        0 | 1 => (20_000, 4),
        2 => (50_000, 8),
        3 => (120_000, 15),
        4 => (250_000, 25),
        _ => (500_000, 40),
    };
    Estimate { tokens, turns }
}

/// The estimate a run of `node` is held against: the card's own `estimate:` line, else the
/// complexity's.
#[must_use]
pub fn estimate_of(node: &Node) -> Estimate {
    node.estimate.unwrap_or_else(|| estimate_for(node.complexity))
}

/// The verdict of a run against its estimate, for the run line and the handoff: `over the
/// estimate of 120k tokens, 15 turns` or `under the estimate of …`.
fn estimate_verdict(estimate: Estimate, usage: &Usage, turns: u64) -> String {
    let side = if estimate.is_over(usage.prompt_tokens + usage.completion_tokens, turns) { "over" } else { "under" };
    format!("{side} the estimate of {estimate}")
}

/// A run stopped because its summed cost crossed its budget. Its text is the note on the node.
#[derive(Debug)]
struct OverBudget {
    budget: f64,
    spent: f64,
    turns: u32,
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
/// the workspace, the results go back, until the model answers in words, the `cap` turns run out
/// when there is one, or the summed cost crosses `budget` when there is one.
async fn tool_loop(
    provider: &dyn ModelProvider,
    worker: &Worker,
    id: i64,
    context: String,
    workspace: Option<&Path>,
    budget: Option<f64>,
    cap: Option<u32>,
) -> anyhow::Result<Session> {
    let workspace = workspace.context("the worker has no workspace to act in")?;
    let toolbox = worker.toolbox(workspace)?;
    let mut messages = vec![Message::system(SYSTEM), Message::user(context)];
    let mut usage = Usage::default();
    let mut turn_usage = Vec::new();
    let mut calls = 0usize;
    let mut turn = 0u32;
    loop {
        turn += 1;
        if let Some(cap) = cap
            && turn > cap
        {
            anyhow::bail!(
                "the worker made {calls} tool call(s) in {cap} turns without finishing; raise the cap with --turns or split the task"
            );
        }
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
                max_turns: cap,
                effort: None,
            })
            .await
            .map_err(map_error)?;
        // every turn is a completion of its own, so each lands in the ledger as it happens
        worker.record(Some(id), &answer.usage, 1).await;
        add_usage(&mut usage, &answer.usage);
        turn_usage.push(TurnUsage::of(&answer.usage));
        if answer.tool_calls.is_empty() {
            return Ok(Session { text: answer.text, usage, calls, turns: u64::from(turn), turn_usage, cap });
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
            max_turns: None,
            effort: None,
        })
        .await
        .map_err(map_error)?;
    worker.record(Some(id), &answer.usage, answer.turns).await;
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
    let mut fields = vec![
        node.objective.clone().unwrap_or_default(),
        node.acceptance.clone().unwrap_or_default(),
        node.notes.as_deref().map(first_line).unwrap_or_default(),
    ];
    for kid in nodes.iter().filter(|n| n.parent == Some(node.id)) {
        fields.push(kid.id.to_string());
        fields.push(kid.state.serde_name().to_owned());
        fields.push(kid.objective.clone().unwrap_or_default());
    }
    hash_fields(fields.iter().map(String::as_str))
}

/// The fingerprint of the card a worker run reads: its objective, its acceptance and its files.
/// It sits beside the session the run made, so a later run tells the same card from a changed one.
#[must_use]
pub fn card_fingerprint(node: &Node) -> String {
    hash_fields([node.objective.as_deref().unwrap_or(""), node.acceptance.as_deref().unwrap_or(""), &node.files.join(" ")])
}

/// Sixteen hex digits of a 64-bit FNV-1a hash over the fields, stable across builds, so a file
/// written today matches tomorrow.
fn hash_fields<'a>(fields: impl IntoIterator<Item = &'a str>) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0100_0000_01b3;
    let mut hash = OFFSET;
    for text in fields {
        // a unit separator after each field, so moving text across fields changes the hash
        for byte in text.bytes().chain(std::iter::once(0x1f)) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
    }
    format!("{hash:016x}")
}

/// How long a session is resumed: past a day its context is older than the work around it.
const SESSION_LIFE: u64 = 24 * 60 * 60;

/// The session a run of `node` resumes at `now`: `Ok(None)` when the node has none, `Err` with
/// the reason when it has one the run must not resume, since a session older than a day or made
/// for other text of the card would lead the harness by what it no longer is.
fn resume_of(node: &Node, now: u64) -> Result<Option<String>, String> {
    let Some(session) = node.session.as_deref().filter(|s| !s.trim().is_empty()) else { return Ok(None) };
    let (Some(of), Some(at)) = (node.session_of.as_deref(), node.session_at) else {
        return Err("the session has no fingerprint".to_owned());
    };
    if of != card_fingerprint(node) {
        return Err("the card changed since the session".to_owned());
    }
    if now.saturating_sub(at) > SESSION_LIFE {
        return Err("the session is older than a day".to_owned());
    }
    Ok(Some(session.to_owned()))
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
pub(crate) fn map_error(error: ProviderError) -> anyhow::Error {
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

#[cfg(test)]
mod tests;
