//! The agent loop: the console's live path to a model. `ping` proves the connection; `run` sends
//! one roadmap task to the badge model and records what came back. The functions talk to a
//! [`ModelProvider`] through a [`Worker`], never to a vendor.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use parking_lot::Mutex;
use ra_providers::local::Role;
use ra_providers::{CompletionRequest, Message, ModelProvider, ProviderError, TurnUsage, Usage};
use ra_roadmap::history::Source;
use ra_roadmap::model::{Effort, Estimate, Gambit, GambitState, Node, NodeKind, TaskState};
use ra_roadmap::store::Store;
use ra_roadmap::tree;
use ra_usage::{Entry, Totals, UsageStore};

/// The project context file, named where the window and the console speak of it.
pub use ra_context::PROJECT_FILE;
use ra_gotchas::{Catalogue, Journal};

use crate::roles::{DIGEST_INSTRUCTION, Roles, SUMMARY_INSTRUCTION};
use crate::secret;

/// The catalogue of the gotchas, beside the roadmap file.
const GOTCHAS_FILE: &str = "var/gotchas.toml";
/// The journal of the gotchas that fired.
const GOTCHAS_LOG: &str = "var/gotchas.log";
/// How many of the latest hits the machine notes of a run draw on.
const RECENT_HITS: usize = 30;
/// The environment variable a run sets to the id of its card, so the hooks the harness calls
/// know which card's context to point at.
pub const CARD_VAR: &str = "RHIPPER_CARD";
/// The files the post hook already answered a whole-file Read on, one `<card>\t<path>` a line;
/// a run starts it afresh, so the hint fires once per file per run.
pub const READ_HINTS_FILE: &str = "var/runs/read-hints";
/// A Read of this many lines or more without a range gets the hint.
const LONG_READ: usize = 400;
/// How many function ranges the hint names.
const HINT_RANGES: usize = 8;

/// The short names of the Claude models and the ids they stand for on the Claude Code route: the
/// ids move with the releases, the names stay, and a person types the name.
pub const ALIASES: [(&str, &str); 4] =
    [("fable", "claude-fable-5-1"), ("opus", "claude-opus-5-5"), ("sonnet", "claude-sonnet-5-5"), ("haiku", ra_providers::route::HAIKU)];

/// The id `model` stands for: the alias table's when it is one of the short names, `model`
/// itself otherwise. Every place a model is typed goes through here, so the badge, the ledger and
/// the harness all see the id.
pub fn resolve(model: &str) -> String {
    let typed = model.trim();
    ALIASES.iter().find(|(alias, _)| alias.eq_ignore_ascii_case(typed)).map_or_else(|| typed.to_owned(), |(_, id)| (*id).to_owned())
}

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
    /// Whether a run picks its model per card: the card's own, else the rule of `rhipper.toml`
    /// that fits, else [`Worker::model`] as the default. Off, every run is on the badge model.
    pub routed: bool,
    /// Whether a run on a `claude-*` model goes ahead past the plan's threshold, from `--force`;
    /// off, such a run falls to the local route when one is up and is refused otherwise.
    pub force: bool,
    /// Debug mode, on unless `RHIPPER_DEBUG=off`: a run asks the worker to end its report with
    /// what would have made it faster, keeps the answer, and hands the latest to the next runs.
    pub debug: bool,
    /// What answers the commands of a run instead of the shell; the tests script the check and
    /// the lint with it.
    script: Option<ra_tools::Script>,
    /// The cheap roles when a test set them; a run resolves its own from `rhipper.toml` and the
    /// local server otherwise.
    pub(crate) roles: Option<Roles>,
    /// The directory the worker acts in and the checks run in when it is not the project's: a
    /// worktree of the card. The roadmap, the context files and `var/` stay the project's.
    workspace: Option<PathBuf>,
    /// The branch of that worktree, which the check line and the handoff name.
    branch: Option<String>,
    /// The verdict a fix pass works from, when the worker is one: it leads the task instead of
    /// the card being taken up afresh.
    fix: Option<String>,
}

impl Worker {
    /// A worker on `model` (an id or one of the [`ALIASES`]), not yet connected and without a
    /// ledger.
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: resolve(&model.into()),
            provider: Arc::new(Mutex::new(None)),
            ledger: None,
            tally: Arc::new(Mutex::new(Totals::default())),
            budget: None,
            budgets: std::env::var("RHIPPER_BUDGETS")
                .is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "on" | "1" | "true" | "yes")),
            turns: None,
            effort: None,
            routed: false,
            force: false,
            debug: !std::env::var("RHIPPER_DEBUG")
                .is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "off" | "0" | "false" | "no")),
            script: None,
            roles: None,
            workspace: None,
            branch: None,
            fix: None,
        }
    }

    /// The same worker, acting in the worktree at `path` on `branch` instead of the project
    /// directory.
    #[must_use]
    pub fn with_worktree(mut self, path: PathBuf, branch: impl Into<String>) -> Self {
        self.workspace = Some(path);
        self.branch = Some(branch.into());
        self
    }

    /// The same worker, with every cheap role on `model` through `provider`; the tests hand in
    /// a scripted local one.
    #[must_use]
    pub fn with_role_provider(mut self, model: impl Into<String>, provider: Arc<dyn ModelProvider>) -> Self {
        self.roles = Some(Roles::preset(model, provider));
        self
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

    /// The same worker, picking its model per card or not.
    #[must_use]
    pub fn with_routing(mut self, on: bool) -> Self {
        self.routed = on;
        self
    }

    /// The same worker, spending the plan past its threshold or not.
    #[must_use]
    pub fn with_force(mut self, on: bool) -> Self {
        self.force = on;
        self
    }

    /// The worker a run in `root` goes to once the plan's week is read: this one, unless it is
    /// on a `claude-*` model and the week is at or past the project's threshold; then, without
    /// `--force`, the same worker on the local server's model when one is up, and an error
    /// naming the room left and the reset when none is.
    ///
    /// # Errors
    /// When the week is past the threshold, the run is not forced and no local route is up.
    pub async fn steered_by_plan(&self, root: &Path) -> anyhow::Result<Self> {
        if self.force || self.route() != Route::ClaudeCode {
            return Ok(self.clone());
        }
        let threshold = plan_threshold(root);
        let readings = plan_readings(root);
        let Some((name, window)) = plan_week(&readings, threshold, unix_now()) else {
            return Ok(self.clone());
        };
        let room = ra_providers::limits::room(&window);
        let at = format!(
            "the plan's week is at {:.0}% as {name} last saw it, past the {:.0}% threshold: {room}",
            window.utilization * 100.0,
            threshold * 100.0
        );
        // the tests never ping the local server
        let local = if cfg!(test) { None } else { ra_providers::local::default_model().await };
        match local {
            Some(model) => {
                tracing::info!(target: crate::trace::PROGRESS, "  steer: {at}; this run goes to {model}");
                let mut worker = self.clone();
                worker.model = model;
                Ok(worker)
            }
            None => anyhow::bail!("{at}; no local route is up, run with --force to spend the week"),
        }
    }

    /// The worker a run of `node` in `root` goes to: this one unless it is routed; then the one
    /// on the model the card, the rules or the default name, with the rule's effort when
    /// `--effort` named none.
    ///
    /// # Errors
    /// When `rhipper.toml` is there but cannot be read.
    pub fn for_card(&self, root: &Path, node: &Node) -> anyhow::Result<Self> {
        if !self.routed {
            return Ok(self.clone());
        }
        let choice = crate::router::Router::load(root)?.choose(node, crate::router::IMPLEMENTER, &self.model);
        let mut worker = self.clone();
        worker.model = resolve(&choice.model);
        worker.effort = self.effort.or(choice.effort);
        Ok(worker)
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

    /// The ledger's totals by model and role, the costliest first, the worker's calls under the
    /// role `None`; `None` as a whole without a ledger, or when it cannot be read.
    pub async fn by_role(&self) -> Option<Vec<(String, Option<String>, Totals)>> {
        let ledger = self.ledger.as_ref()?;
        match ledger.by_role().await {
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
        self.record_as(&self.model, None, node, usage, turns).await;
    }

    /// [`record`](Self::record) under `model` rather than the badge's and tagged with `role`: a
    /// cheap role's call lands under the model that took it, so the saving shows in `rc usage`
    /// beside the worker's calls.
    pub(crate) async fn record_as(&self, model: &str, role: Option<&str>, node: Option<i64>, usage: &Usage, turns: u64) {
        let entry = Entry {
            model: model.to_owned(),
            role: role.map(str::to_owned),
            node,
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            cached_tokens: usage.cached_tokens,
            cost_usd: usage.cost_usd,
            cost_computed: usage.cost_computed,
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
        let provider = connect(route)?;
        *slot = Some((route, provider.clone()));
        Ok(provider)
    }

    /// The provider of `model`, which a cheap role runs on: the badge's own when the routes
    /// agree, else a fresh connection on the model's route.
    pub(crate) fn provider_for(&self, model: &str) -> anyhow::Result<Arc<dyn ModelProvider>> {
        let route = Route::of(model);
        if route == self.route() { self.provider() } else { connect(route) }
    }

    /// Drops the connected provider, so the next call connects with the current key.
    pub fn forget(&self) {
        *self.provider.lock() = None;
    }
}

/// A fresh connection on `route`, with what each needs: the aggregator the key from the
/// credential store or the environment, and a missing one is the error; Claude Code only its own
/// login; the local server no key, only to be running.
fn connect(route: Route) -> anyhow::Result<Arc<dyn ModelProvider>> {
    let provider: Arc<dyn ModelProvider> = match route {
        Route::ClaudeCode => {
            // the gotchas guard every harness run: the hooks call this very binary, and the
            // catalogue lives beside the roadmap; without either, the harness runs unguarded
            let mut harness = ra_providers::claude_code::ClaudeCode::new();
            // the project's own commands and environment, from the `[worker]` table of
            // its rhipper.toml, when it has one
            match crate::router::Router::load(Path::new(".")) {
                Ok(router) => {
                    let worker = router.worker;
                    if !worker.allow.is_empty() {
                        harness = harness.with_allowed(worker.rules());
                    }
                    if !worker.env.is_empty() {
                        harness = harness.with_env(worker.env);
                    }
                }
                Err(e) => tracing::warn!("the worker table is off: {e:#}"),
            }
            // the project's catalogue when it has one, brought what the seed gained since it
            // was written; the seed otherwise, written by the hook in the run's own directory,
            // not here (a test runs where no var/ belongs)
            let catalogue = if Path::new(GOTCHAS_FILE).exists() {
                Catalogue::refresh(Path::new(GOTCHAS_FILE)).map(|(catalogue, added)| {
                    if !added.is_empty() {
                        tracing::info!("{GOTCHAS_FILE} gained {} from the seed", added.join(", "));
                    }
                    catalogue
                })
            } else {
                Ok(Catalogue::seed())
            };
            match (std::env::current_exe(), catalogue) {
                (Ok(rc), Ok(catalogue)) => harness = harness.with_gotchas(rc, catalogue),
                (_, Err(e)) => tracing::warn!("the gotchas are off: {e:#}"),
                (Err(e), _) => tracing::warn!("the gotchas are off: no path to this binary: {e}"),
            }
            Arc::new(harness)
        }
        // the prices of the day in the project's var/, for the answers that come back uncosted
        Route::OpenRouter => {
            Arc::new(ra_providers::openrouter::OpenRouter::new(secret::api_key()?)?.with_prices_at(ra_providers::openrouter::PRICES_FILE))
        }
        // the same wire format as OpenRouter, at the local URL and with no key
        Route::Local => Arc::new(ra_providers::openrouter::OpenRouter::local()?),
    };
    Ok(provider)
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

/// One line of `rc doctor`: what was looked at, whether it holds, whether a run needs it, and
/// what was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub required: bool,
    pub detail: String,
}

impl Check {
    fn new(name: &'static str, required: bool, outcome: Result<String, String>) -> Self {
        match outcome {
            Ok(detail) => Self { name, ok: true, required, detail },
            Err(detail) => Self { name, ok: false, required, detail },
        }
    }

    /// The checks as a table: a mark, the name and what was found. A failed optional check gets
    /// a dash rather than a cross, since it blocks no run.
    pub fn table(checks: &[Self]) -> Vec<String> {
        let mut lines = vec![format!("   {:<15} found", "check")];
        for check in checks {
            let mark = match (check.ok, check.required) {
                (true, _) => "✓",
                (false, true) => "✗",
                (false, false) => "–",
            };
            lines.push(format!("{mark}  {:<15} {}", check.name, check.detail));
        }
        lines
    }

    /// The names of the required checks that failed; a run needs every one of them.
    pub fn blocking(checks: &[Self]) -> Vec<&'static str> {
        checks.iter().filter(|c| c.required && !c.ok).map(|c| c.name).collect()
    }
}

/// How long a probe of a program or a server may take before the doctor gives up on it.
const DOCTOR_PROBE: std::time::Duration = std::time::Duration::from_secs(30);

/// What a run in `root` needs, each looked at once: the harness and its login, the hooks and
/// the settings a worker gets, git, the two other routes, the roadmap and the ledger. The
/// routes a run can do without are optional; the rest is required.
pub async fn doctor(root: &Path) -> Vec<Check> {
    vec![
        Check::new("claude", true, claude_check(root).await),
        Check::new("hooks", false, hooks_check(root)),
        Check::new("settings", false, settings_check(root)),
        Check::new("git", true, git_check(root).await),
        Check::new("local server", false, local_check().await),
        Check::new("OpenRouter key", false, key_check()),
        Check::new("roadmap", true, roadmap_check(root)),
        Check::new("ledger", true, ledger_check(root).await),
    ]
}

/// One run of `program` with `args` in `dir`, within the doctor's patience; the error names
/// what went wrong with the program itself, not what it printed.
async fn probe(program: &str, args: &[&str], dir: &Path) -> Result<std::process::Output, String> {
    let mut command = tokio::process::Command::new(program);
    // a nested session refuses to start; the doctor may itself run under Claude Code
    command.args(args).current_dir(dir).env_remove("CLAUDECODE");
    match tokio::time::timeout(DOCTOR_PROBE, command.output()).await {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => Err(format!("`{program}` is not on the path")),
        Ok(Err(e)) => Err(format!("cannot start `{program}`: {e}")),
        Err(_) => Err(format!("`{program} {}` gave no answer in {}s", args.join(" "), DOCTOR_PROBE.as_secs())),
    }
}

/// The first line of what a program printed, stderr when stdout is empty.
fn printed_line(output: &std::process::Output) -> String {
    let text = if output.stdout.iter().any(|b| !b.is_ascii_whitespace()) { &output.stdout } else { &output.stderr };
    String::from_utf8_lossy(text).lines().next().unwrap_or_default().trim().to_owned()
}

/// `claude` on the path and logged in: its version, and what `claude auth status` says.
async fn claude_check(root: &Path) -> Result<String, String> {
    let version = probe("claude", &["--version"], root).await?;
    if !version.status.success() {
        return Err(format!("`claude --version` failed: {}", printed_line(&version)));
    }
    let version = printed_line(&version);
    let status = probe("claude", &["auth", "status"], root).await?;
    // the status is JSON when the harness knows the subcommand, with `loggedIn` as its word;
    // an older harness without it is believed on its exit code
    let text = String::from_utf8_lossy(&status.stdout);
    let json = serde_json::from_str::<serde_json::Value>(text.trim()).ok();
    let logged_in = json.as_ref().and_then(|v| v.get("loggedIn")).and_then(serde_json::Value::as_bool);
    let method = json.as_ref().and_then(|v| v.get("authMethod")).and_then(serde_json::Value::as_str).unwrap_or("the harness");
    match logged_in {
        Some(true) => Ok(format!("{version}, logged in through {method}")),
        Some(false) => Err(format!("{version}, not logged in: run `claude` once and log in")),
        None if status.status.success() => Ok(format!("{version}, logged in")),
        None => Err(format!("{version}; `claude auth status` failed: {}", printed_line(&status))),
    }
}

/// The hooks reachable: this binary, which the harness calls as `rc hook pre|post`, and the
/// catalogue they answer from.
fn hooks_check(root: &Path) -> Result<String, String> {
    let rc = std::env::current_exe().map_err(|e| format!("no path to this binary, so the harness runs unguarded: {e}"))?;
    if !rc.is_file() {
        return Err(format!("{} is gone, so the harness runs unguarded", rc.display()));
    }
    let file = root.join(GOTCHAS_FILE);
    let (catalogue, source) = if file.is_file() {
        (
            Catalogue::load(&file).map_err(|e| format!("{} does not load, so the harness runs unguarded: {e:#}", file.display()))?,
            GOTCHAS_FILE,
        )
    } else {
        (Catalogue::seed(), "the seed catalogue")
    };
    Ok(format!("`{}` hook pre|post, {} gotchas from {source}", rc.display(), catalogue.entries.len()))
}

/// The settings a worker loads, and which of the project's files are there to load.
fn settings_check(root: &Path) -> Result<String, String> {
    let presence = |file: &str| if root.join(".claude").join(file).is_file() { "present" } else { "absent" };
    Ok(format!(
        "a worker loads {}: .claude/settings.json {}, .claude/settings.local.json {}; the user's hooks stay out",
        ra_providers::claude_code::WORKER_SETTING_SOURCES,
        presence("settings.json"),
        presence("settings.local.json")
    ))
}

/// Marks the start of a run of card `id` for the hooks: the card id in this process's
/// environment, which the harness and its hooks inherit, and the read hints of the last run
/// forgotten.
fn begin_run(root: &Path, id: i64) {
    // SAFETY: the console runs one card at a time and reads no environment on another thread
    // while a run starts
    unsafe { std::env::set_var(CARD_VAR, id.to_string()) };
    let _ = std::fs::remove_file(root.join(READ_HINTS_FILE));
}

/// The hint the post hook answers a whole-file Read with: for a `Read` of a file of
/// [`LONG_READ`] lines or more without an offset or a limit, the card's outline (`rc context
/// <id>`) and the line ranges of the functions the card names, so the next Read takes a range.
/// Nothing for a ranged Read, a short file, or a file this run was already told about. The card
/// is the one in [`CARD_VAR`], else the newest context file under `var/context/`.
#[must_use]
pub fn read_hint(root: &Path, call: &serde_json::Value) -> Option<String> {
    hint_of(root, call, card_of_run(root))
}

/// [`read_hint`] for a known card, so a test is not at the mercy of the [`CARD_VAR`] a run
/// around it set; `None` for the card is no hint at all.
fn hint_of(root: &Path, call: &serde_json::Value, card: Option<i64>) -> Option<String> {
    use serde_json::Value;
    if call.get("tool_name").and_then(Value::as_str) != Some("Read") {
        return None;
    }
    let input = call.get("tool_input")?;
    if !input.get("offset").is_none_or(Value::is_null) || !input.get("limit").is_none_or(Value::is_null) {
        return None;
    }
    let file = input.get("file_path").and_then(Value::as_str)?;
    let path = if Path::new(file).is_absolute() { PathBuf::from(file) } else { root.join(file) };
    let text = std::fs::read_to_string(&path).ok()?;
    if text.lines().count() < LONG_READ {
        return None;
    }
    let card = card?;
    let shown = path.strip_prefix(root).unwrap_or(&path).display().to_string().replace('\\', "/");
    if !first_hint(root, card, &shown) {
        return None;
    }
    // the functions the card names come first; a card that names none gets the file's head
    let brief = std::fs::read_to_string(ra_context::TaskContext::path(root, card)).unwrap_or_default();
    let items = ra_context::items(&path, &text);
    let named: Vec<&(usize, usize, String)> = items
        .iter()
        .filter(|(_, _, item)| ra_context::item_name(item).is_some_and(|name| name.len() > 2 && brief.contains(name)))
        .collect();
    let picked: Vec<&(usize, usize, String)> = if named.is_empty() { items.iter().collect() } else { named };
    let ranges: Vec<String> = picked
        .iter()
        .take(HINT_RANGES)
        .map(|(start, end, item)| format!("{} {start}-{end}", ra_context::item_name(item).unwrap_or(item)))
        .collect();
    let mut hint =
        format!("{shown} is {} lines: read it by range (offset, limit). Its outline is in `rc context {card}`", text.lines().count());
    if !ranges.is_empty() {
        hint.push_str(&format!("; the ranges: {}", ranges.join(", ")));
    }
    Some(hint)
}

/// The card a hook is running under: [`CARD_VAR`] when the run set it, else the newest file of
/// `var/context/`, the one the run just wrote.
fn card_of_run(root: &Path) -> Option<i64> {
    if let Some(id) = std::env::var(CARD_VAR).ok().and_then(|v| v.trim().parse().ok()) {
        return Some(id);
    }
    std::fs::read_dir(root.join(ra_context::CONTEXT_DIR))
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let id: i64 = path.file_stem()?.to_str()?.parse().ok()?;
            Some((entry.metadata().ok()?.modified().ok()?, id))
        })
        .max()
        .map(|(_, id)| id)
}

/// Whether `file` was not yet hinted for `card` this run, noting it so the next ask says no.
fn first_hint(root: &Path, card: i64, file: &str) -> bool {
    let memo = root.join(READ_HINTS_FILE);
    let line = format!("{card}\t{file}");
    if std::fs::read_to_string(&memo).is_ok_and(|text| text.lines().any(|l| l == line)) {
        return false;
    }
    if let Some(dir) = memo.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut text = std::fs::read_to_string(&memo).unwrap_or_default();
    text.push_str(&line);
    text.push('\n');
    let _ = std::fs::write(&memo, text);
    true
}

/// git on the path, and whether `root` is a repository: the review reads the diff and the
/// worktrees branch from HEAD.
async fn git_check(root: &Path) -> Result<String, String> {
    let version = probe("git", &["--version"], root).await?;
    if !version.status.success() {
        return Err(format!("`git --version` failed: {}", printed_line(&version)));
    }
    let repository = probe("git", &["rev-parse", "--is-inside-work-tree"], root).await.is_ok_and(|o| o.status.success());
    let place = if repository { "a repository here" } else { "no repository here: the review has no diff and a worktree no HEAD" };
    Ok(format!("{}, {place}", printed_line(&version)))
}

/// The local server answering at its URL, and the model the roles default to on it.
async fn local_check() -> Result<String, String> {
    let url = ra_providers::openrouter::local_url();
    match ra_providers::local::default_model_at(&url).await {
        Some(model) => Ok(format!("{url} answers; the roles default to {model}")),
        None => {
            Err(format!("nothing answers at {url}; set {} to a server, or the roles call nothing", ra_providers::openrouter::LOCAL_URL_VAR))
        }
    }
}

/// The OpenRouter key, masked, and where it came from.
fn key_check() -> Result<String, String> {
    let source =
        if std::env::var("OPENROUTER_API_KEY").is_ok_and(|k| !k.trim().is_empty()) { "OPENROUTER_API_KEY" } else { "the credential store" };
    secret::masked().map(|mask| format!("{mask} from {source}")).map_err(|e| format!("{e:#}; the OpenRouter route is off"))
}

/// The roadmap file parsing, and how many nodes it holds.
fn roadmap_check(root: &Path) -> Result<String, String> {
    let path = root.join(crate::setup::ROADMAP_FILE);
    let text = std::fs::read_to_string(&path).map_err(|_| format!("no {}; `rc init` makes one", path.display()))?;
    let nodes = ra_roadmap::markdown::parse(&text).map_err(|e| format!("{} does not parse: {e:#}", path.display()))?;
    Ok(format!("{} parses, {} nodes", path.display(), nodes.len()))
}

/// The ledger opening under `var/`, and how many calls it holds.
async fn ledger_check(root: &Path) -> Result<String, String> {
    let dir = root.join("var");
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
    let path = dir.join("usage.db");
    let ledger = UsageStore::open(&path).await.map_err(|e| format!("{} does not open: {e:#}", path.display()))?;
    let totals = ledger.totals().await.map_err(|e| format!("{} does not read: {e:#}", path.display()))?;
    Ok(format!("{} opens, {} calls", path.display(), totals.calls))
}

/// Runs one roadmap task: builds the context from the node, its ancestors and its dependencies,
/// sends it to the model, and writes the answer and the state back. A failed call leaves the node
/// failed, not in progress. The run announces its start and its outcome on the progress channel,
/// so the console draws its steps in a bubble of their own.
pub async fn run(store: &Store, id: i64, worker: &Worker) -> anyhow::Result<Vec<String>> {
    run_picked(store, id, worker, false).await
}

/// [`run`], with whether a tight window picked the card, which the start line and the pair
/// feed then say.
async fn run_picked(store: &Store, id: i64, worker: &Worker, tight: bool) -> anyhow::Result<Vec<String>> {
    // the node first, so a bad id or a project is named before the key matters
    let node = store.read(id).await?;
    anyhow::ensure!(node.kind != NodeKind::Project, "#{} is a project; run a task", id);
    // the model of this card, when the command left the choice to the card and the rules
    let root = workspace_of(store);
    // then the plan's week: a claude-* pick past the threshold falls to the local route or is
    // refused here, before the start marker
    let worker = &worker.for_card(&root, &node)?.steered_by_plan(&root).await?;
    let pick = if tight { format!(" · {TIGHT_PICK}") } else { String::new() };
    // the pid on the start marker, last: a window coming up tells a run that died from one
    // still going
    tracing::info!(
        target: crate::trace::PROGRESS,
        "{}#{} {} · {}{pick} · pid {}",
        crate::logs::RUN_START,
        id,
        node.title,
        short(&worker.model),
        std::process::id()
    );
    // the check and the lint the run ends on, resolved up front: the activity log and the paired
    // session see what the card will be judged by before the worker starts
    let checks = checks_of(&root, &node).describe();
    tracing::info!(target: crate::trace::PROGRESS, "{checks}");
    // the paired session hears of the start and the end through the feed it watches
    let title = node.title.clone();
    // the pid last on the feed's start too: `rc status` tells a live run from an orphan by it
    crate::pair::event(&root, &format!("start #{id} {title} · {}{pick} · {checks} · pid {}", short(&worker.model), std::process::id()));
    // the plan's room across every project on the machine, before any token goes
    if worker.route() == Route::ClaudeCode {
        wait_for_plan(&root).await;
    }
    // the other projects see the run while it goes, and what it came to once it ended
    let started = unix_now();
    write_digest(store, &root, Some(format!("#{id} {title} · pid {} · started {}", std::process::id(), clock(started))), None, None).await;
    // the cheap roles of this run, resolved once: the local server is pinged here, not per role
    let roles = worker.roles(&root).await;
    let mut outcome = run_task(store, node, worker, &roles).await;
    // the reviewer judges a run that passed its checks before the card is offered: implemented
    // means review-ready, and a card is review-ready only with a verdict attached
    if let Ok((lines, _, _)) = &mut outcome
        && store.read(id).await.is_ok_and(|n| n.state == TaskState::Implemented)
    {
        lines.extend(review_after_run(store, id, worker).await);
    }
    match &outcome {
        Ok((lines, cost, 0)) => {
            tracing::info!(target: crate::trace::PROGRESS, "{}✓ {}", crate::logs::RUN_END, cost_text(*cost));
            crate::pair::event(&root, &format!("end {}", lines.first().map_or("", String::as_str)));
        }
        Ok((lines, cost, gotchas)) => {
            tracing::info!(target: crate::trace::PROGRESS, "{}✓ {} · {gotchas} gotchas", crate::logs::RUN_END, cost_text(*cost));
            crate::pair::event(&root, &format!("end {} · {gotchas} gotcha(s)", lines.first().map_or("", String::as_str)));
        }
        Err(e) => {
            tracing::info!(target: crate::trace::PROGRESS, "{}✗ {}", crate::logs::RUN_END, first_line(&format!("{e:#}")));
            crate::pair::event(&root, &format!("end ✗ #{id} {title} failed: {}", first_line(&format!("{e:#}"))));
        }
    }
    let changed = ra_context::TaskContext::open(&root, id).handoffs.last().map(|h| h.changed.clone()).unwrap_or_default();
    // the digest role drafts the run's line for the other projects from the end line and the
    // notes of the card, when it has a model; the feed's end line stands otherwise
    let drafted = match crate::pair::last_end(&root) {
        Some(end) => {
            let notes = store.read(id).await.ok().and_then(|n| n.notes).unwrap_or_default();
            let notes: String = notes.chars().take(DIGEST_NOTES).collect();
            worker
                .ask_role(&roles, Role::Digest, Some(id), DIGEST_INSTRUCTION, &format!("{end}\n\n{notes}"))
                .await
                .map(|line| first_line(&line))
        }
        None => None,
    };
    write_digest(store, &root, None, Some(changed), drafted).await;
    outcome.map(|(lines, _, _)| lines)
}

/// What the reviewer is told: the verdict on the first line, the points after it.
const REVIEW_INSTRUCTION: &str = "You review one card of a roadmap after a worker implemented it. You get the card's objective and acceptance, the worker's handoff, and the git diff of the card's files. Judge whether the diff meets the acceptance and whether the handoff tells the truth about it. Answer with one word on the first line: `accepted` or `needs_revision`. After `needs_revision`, list the points as `- ` lines, each naming the file and what is wrong or missing; after `accepted`, nothing or one line of why. No other text.";

/// How much of the diff the reviewer reads: a card's files, not a reformat of the tree.
const REVIEW_DIFF: usize = 60_000;

/// The reviewer's verdict on a card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Accepted,
    /// The points the reviewer listed, one per entry, without their bullets.
    NeedsRevision(Vec<String>),
}

/// The verdict of a reviewer's answer: `accepted` on the first line is acceptance; anything
/// else is a revision, with the lines after a `needs_revision` first line as its points, or
/// every line when the first is not the word.
#[must_use]
pub fn verdict_of(answer: &str) -> Verdict {
    let mut lines = answer.lines().map(str::trim).filter(|line| !line.is_empty());
    let Some(first) = lines.next() else {
        return Verdict::NeedsRevision(vec!["the reviewer answered nothing".to_owned()]);
    };
    let word = first.trim_matches(|c: char| !c.is_alphanumeric()).to_ascii_lowercase().replace(' ', "_");
    if word.starts_with("accepted") {
        return Verdict::Accepted;
    }
    let point = |line: &str| line.trim_start_matches(['-', '*', '•']).trim().to_owned();
    let points: Vec<String> = if word.starts_with("needs_revision") {
        lines.map(point).collect()
    } else {
        std::iter::once(first).chain(lines).map(point).collect()
    };
    Verdict::NeedsRevision(if points.is_empty() { vec!["the reviewer gave no points".to_owned()] } else { points })
}

/// What a fix pass is told, ahead of the verdict and the card.
const FIX_INSTRUCTION: &str = "A reviewer sent this card back for the points below, every one of them a lint warning, a format diff or a doc comment. Fix exactly those points in the files they name, run the card's checks, and report in a few lines what you changed; change nothing else, and do not take the card up afresh.";

/// The outcome a review leaves on the handoff of a trivially rejected run; a second one on the
/// same card halts the fix passes.
const TRIVIAL_TAG: &str = "needs revision (trivial)";

/// Has a second model review a card: it reads the objective and the acceptance, the last
/// handoff and the `git diff` of the card's files (and of the ones the handoff names), and
/// gives a verdict. The card is in review while the reviewer reads; it ends accepted, or needs
/// revision with the points first in its notes. The reviewer's model is the `reviewer` rule of
/// `rhipper.toml` that fits the card, else the worker's own. Every run whose checks pass has
/// this judgment before the card is offered ([`review_after_run`]); this is the person's own
/// call for it.
///
/// A trivial rejection (lint, format, doc comment) is not left for the person: one fix pass
/// runs on the cheap route ([`ra_providers::route`]) with the verdict as its task, the card's
/// checks run again, and the reviewer judges the card once more. A second trivial rejection of
/// the same card, a fix pass whose check failed, or a `fix = ""` in the `[role]` table halts
/// there, and the card waits for the person.
///
/// # Errors
/// A bad id or a project, an unreadable `rhipper.toml`, no provider for the model, or a failed
/// call; the card goes back to the state it had.
pub async fn review(store: &Store, id: i64, worker: &Worker) -> anyhow::Result<Vec<String>> {
    review_to(store, id, worker, TaskState::Accepted).await
}

/// The reviewer on a card a run just left implemented, before it is offered for acceptance:
/// [`review`] with an accepting verdict leaving the card implemented, the verdict on its
/// handoff, so acceptance is the person's glance at it; a rejection sets needs revision as
/// ever. A reviewer that does not answer leaves the card in review, without a verdict and so
/// not offered: `review <id>` judges it once the reviewer answers. The lines of the verdict.
async fn review_after_run(store: &Store, id: i64, worker: &Worker) -> Vec<String> {
    let root = workspace_of(store);
    match review_to(store, id, worker, TaskState::Implemented).await {
        Ok(lines) => {
            if let Some(first) = lines.first() {
                crate::pair::event(&root, &format!("review {first}"));
            }
            lines
        }
        Err(error) => {
            let why = first_line(&format!("{error:#}"));
            if let Err(e) = store.set_state_by(id, TaskState::Review, Source::Run).await {
                tracing::warn!("#{id} could not be held in review: {e:#}");
            }
            crate::pair::event(&root, &format!("review #{id} no verdict: {why}"));
            vec![format!("  no verdict on #{id}: {why} · the card waits in review; `review {id}` judges it")]
        }
    }
}

/// [`review`], with the state an accepting verdict leaves the card in: accepted when the person
/// asked for the judgment, implemented when a run did, so the card is offered to the person with
/// the verdict attached rather than accepted behind their back.
async fn review_to(store: &Store, id: i64, worker: &Worker, accepted: TaskState) -> anyhow::Result<Vec<String>> {
    let (mut lines, verdict) = judge(store, id, worker, accepted).await?;
    let Some(verdict) = verdict else {
        return Ok(lines);
    };
    let root = workspace_of(store);
    // the handoffs count the trivial rejections of the card: the fix pass runs on the first
    // alone, so a pass that did not satisfy the reviewer is not repeated blind
    let rejected = ra_context::TaskContext::open(&root, id).handoffs.iter().filter(|h| h.outcome == TRIVIAL_TAG).count();
    if rejected >= 2 {
        lines.push("  a second trivial rejection: no fix pass, the card waits for the person".to_owned());
        return Ok(lines);
    }
    let named = crate::router::Router::load(&root).ok().and_then(|router| router.role.fix);
    let model = match named.as_deref() {
        Some(named) => ra_providers::route::cheap(Some(named), None),
        // the tests neither ping the local server nor fall to haiku; a run does, once
        None if cfg!(test) => None,
        None => ra_providers::route::cheap_model(None).await,
    };
    let Some(model) = model else {
        lines.push("  the fix role is off: the card waits for the person".to_owned());
        return Ok(lines);
    };
    let fixer = Worker { model: resolve(&model), fix: Some(verdict), ..worker.clone() };
    tracing::info!(target: crate::trace::PROGRESS, "#{id} · fix pass on {}", short(&fixer.model));
    lines.push(format!("  fix pass on {}", short(&fixer.model)));
    let node = store.read(id).await?;
    let roles = worker.roles(&root).await;
    match run_task(store, node, &fixer, &roles).await {
        Ok((fixed, _, _)) => lines.extend(fixed),
        Err(e) => {
            lines.push(format!("  the fix pass failed: {}; the card waits for the person", first_line(&format!("{e:#}"))));
            return Ok(lines);
        }
    }
    // the reviewer judges the fixed card only when its checks passed; a failed check is the
    // person's to read either way
    if store.read(id).await?.state != TaskState::Implemented {
        lines.push("  the fix pass did not pass the check: the card waits for the person".to_owned());
        return Ok(lines);
    }
    let (judged, again) = judge(store, id, worker, accepted).await?;
    lines.extend(judged);
    if again.is_some() {
        lines.push("  a second trivial rejection: the card waits for the person".to_owned());
    }
    Ok(lines)
}

/// One verdict of the reviewer on a card, as [`review`] describes it, without the fix pass:
/// the lines, and the head of the notes (the verdict with its points) when the rejection was
/// trivial. `accepted` is where an accepting verdict leaves the card ([`review_to`]).
async fn judge(store: &Store, id: i64, worker: &Worker, accepted: TaskState) -> anyhow::Result<(Vec<String>, Option<String>)> {
    let node = store.read(id).await?;
    anyhow::ensure!(node.kind != NodeKind::Project, "#{id} is a project; review a task");
    let root = workspace_of(store);
    // the reviewer is a role of its own: the card's assignee is its implementer, not its judge
    let model = match crate::router::Router::load(&root)?.pick(&node, crate::router::REVIEWER) {
        Some((_, rule)) => resolve(&rule.model),
        None => worker.model.clone(),
    };
    let provider = worker.provider_for(&model)?;
    let brief = review_brief(&root, &node).await;
    let before = node.state;
    store.set_state_by(id, TaskState::Review, Source::Run).await?;
    tracing::info!(target: crate::trace::PROGRESS, "#{id} {} · review by {}", node.title, short(&model));
    let answer = provider
        .complete(CompletionRequest {
            model: model.clone(),
            messages: vec![Message::system(REVIEW_INSTRUCTION), Message::user(&brief)],
            temperature: Some(0.0),
            max_tokens: Some(1500),
            tools: Vec::new(),
            workspace: None,
            budget_usd: None,
            resume: None,
            max_turns: Some(1),
            effort: Some(Effort::default().to_string()),
        })
        .await;
    let answer = match answer {
        Ok(answer) => answer,
        Err(error) => {
            // a reviewer that did not answer is no verdict: the card goes back where it was
            store.set_state_by(id, before, Source::Run).await?;
            return Err(map_error(error));
        }
    };
    worker.record_as(&model, Some("reviewer"), Some(id), &answer.usage, answer.turns).await;
    let verdict = verdict_of(&answer.text);
    let mut trivial = None;
    let (state, head, said) = match &verdict {
        Verdict::Accepted => (accepted, format!("review by {model}: accepted"), "accepted".to_owned()),
        Verdict::NeedsRevision(points) => {
            let weight = crate::commands::review::classify(points);
            let list: Vec<String> = points.iter().map(|point| format!("- {point}")).collect();
            let head = format!("review by {model}: needs revision ({})\n{}", weight.name(), list.join("\n"));
            if weight == crate::commands::review::Weight::Trivial {
                trivial = Some(head.clone());
            }
            (TaskState::NeedsRevision, head, format!("needs revision ({}) · {} point(s)", weight.name(), points.len()))
        }
    };
    // the verdict rides the run's handoff: a card is offered for acceptance only with one
    // attached, and the weight of a rejection retags the outcome, a trivial one for a cheap
    // pass, a substantive one for the person. A card reviewed before any run has no handoff,
    // and the notes carry the verdict alone
    let mut task = ra_context::TaskContext::open(&root, id);
    if let Some(last) = task.handoffs.last_mut() {
        last.verdict = Some(format!("{said} by {model}"));
        if state == TaskState::NeedsRevision
            && let Some((tag, _)) = said.split_once(" · ")
        {
            tag.clone_into(&mut last.outcome);
        }
        if let Err(e) = task.save(&root) {
            tracing::warn!("the verdict of #{id} was not saved on its handoff: {e:#}");
        }
    }
    // the verdict leads the notes when it is the next reader's task: a rejection's points for
    // the next run, or an acceptance the person asked for. An acceptance after a run leaves the
    // notes to the worker's report; the handoff carries it
    if state != TaskState::Implemented {
        let notes = match node.notes.as_deref().map(str::trim) {
            Some(report) if !report.is_empty() => format!("{head}\n\n{report}"),
            _ => head,
        };
        store.set_notes(id, &notes).await?;
    }
    store.set_state_by(id, state, Source::Run).await?;
    let mut lines = vec![format!(
        "{} #{} {} {said} by {} · in {} out {} · {}",
        if verdict == Verdict::Accepted { "✓" } else { "✗" },
        id,
        node.title,
        short(&model),
        answer.usage.prompt_tokens,
        answer.usage.completion_tokens,
        cost_text(answer.usage.cost_usd)
    )];
    if let Verdict::NeedsRevision(points) = &verdict {
        lines.extend(points.iter().map(|point| format!("  - {point}")));
    }
    Ok((lines, trivial))
}

/// What the reviewer reads: the card, the last handoff when there is one, and the diff of the
/// card's files and the handoff's changed ones against `HEAD`, clipped.
async fn review_brief(root: &Path, node: &Node) -> String {
    let mut text = format!("# #{} {}\n", node.id, node.title);
    if let Some(objective) = &node.objective {
        let _ = writeln!(text, "\nObjective: {objective}");
    }
    let _ = writeln!(text, "\nAcceptance: {}", node.acceptance.as_deref().unwrap_or("none written; the workspace tests are the check"));
    let mut files = node.files.clone();
    match ra_context::TaskContext::open(root, node.id).handoffs.last() {
        Some(last) => {
            let _ = writeln!(text, "\n## Handoff of the last run\n{} on {} · {}\n{}", last.outcome, last.model, last.check, last.report);
            for file in &last.changed {
                if !files.contains(file) {
                    files.push(file.clone());
                }
            }
        }
        None => text.push_str("\n## Handoff of the last run\nnone: no run of this card left one\n"),
    }
    // the notes hold the worker's report whole where the handoff holds its summary
    if let Some(notes) = node.notes.as_deref().filter(|notes| !notes.trim().is_empty()) {
        let notes: String = notes.chars().take(DIGEST_NOTES).collect();
        let _ = writeln!(text, "\n## Notes of the card\n{notes}");
    }
    text.push_str("\n## git diff HEAD of the card's files\n");
    if files.is_empty() {
        text.push_str("the card names no files and no run changed any\n");
        return text;
    }
    let mut command = tokio::process::Command::new("git");
    command.args(["diff", "HEAD", "--"]).args(&files).current_dir(root);
    let diff = match command.output().await {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).into_owned(),
        Ok(out) => format!("git diff failed: {}", String::from_utf8_lossy(&out.stderr).trim()),
        Err(e) => format!("git diff did not run: {e}"),
    };
    if diff.trim().is_empty() {
        let _ = writeln!(text, "no change against HEAD in {}", files.join(", "));
    } else if diff.chars().count() > REVIEW_DIFF {
        let cut: String = diff.chars().take(REVIEW_DIFF).collect();
        let _ = writeln!(text, "{cut}\n… clipped at {REVIEW_DIFF} characters");
    } else {
        text.push_str(&diff);
    }
    text
}

/// What the planner is told: the cards of one milestone or section, as a plain outline the store
/// imports as it is.
const PLAN_INSTRUCTION: &str = "You are the architect of a roadmap. You get the project context, one milestone or section with its objective and acceptance, and the cards already under it. Split the work it still owes into cards a worker can each finish in one sitting. Answer with a plain outline and nothing else: one `- Title {cN}` line per card, N the complexity 1..5, followed by indented `objective:`, `acceptance:` and `files:` lines; the acceptance names the command that checks the card in backticks; the files are the paths the change lands in, separated by spaces. No headings, no card that repeats one already listed, no prose.";

/// The cards the planner proposed for a milestone or a section: shown first, added on the
/// person's word.
#[derive(Debug, Clone)]
pub struct Plan {
    /// The milestone or the section the cards go under.
    pub id: i64,
    pub title: String,
    /// The planner's model.
    pub model: String,
    pub outline: ra_roadmap::outline::Outline,
    pub usage: Usage,
}

impl Plan {
    /// The proposal as lines: the head with the usage, then the cards as an import would nest
    /// them.
    #[must_use]
    pub fn preview(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "{} proposed under #{} {} by {} · in {} out {} · {}",
            self.outline.summary(),
            self.id,
            self.title,
            short(&self.model),
            self.usage.prompt_tokens,
            self.usage.completion_tokens,
            cost_text(self.usage.cost_usd)
        )];
        lines.extend(self.outline.preview().into_iter().map(|line| format!("  {line}")));
        lines
    }

    /// Adds the cards to the store, defined with their fields; one line per card added.
    ///
    /// # Errors
    /// When the store cannot take them.
    pub async fn add(&self, store: &Store) -> anyhow::Result<Vec<String>> {
        let ids = store.import(self.id, &self.outline).await?;
        let nodes = store.all().await?;
        let mut lines: Vec<String> = ids
            .iter()
            .filter_map(|id| nodes.iter().find(|n| n.id == *id))
            .map(|node| format!("#{} {} {} {{c{}}}", node.id, node.kind, node.title, node.complexity))
            .collect();
        lines.push(format!("added {} under #{} {}", self.outline.summary(), self.id, self.title));
        Ok(lines)
    }
}

/// Asks the planner to split a milestone or a section into cards and returns them, adding
/// nothing: the caller shows the plan and adds it on confirmation. The planner's model is the
/// `planner` rule of `rhipper.toml` that fits the node, else the worker's own.
///
/// # Errors
/// A bad id or a node that is not a milestone or a section, an unreadable `rhipper.toml`, no
/// provider for the model, a failed call, or an answer with no card in it.
pub async fn plan(store: &Store, id: i64, worker: &Worker) -> anyhow::Result<Plan> {
    let node = store.read(id).await?;
    anyhow::ensure!(
        matches!(node.kind, NodeKind::Milestone | NodeKind::Section),
        "#{id} is a {}; plan a milestone or a section",
        node.kind
    );
    let root = workspace_of(store);
    let model = match crate::router::Router::load(&root)?.pick(&node, crate::router::PLANNER) {
        Some((_, rule)) => resolve(&rule.model),
        None => worker.model.clone(),
    };
    let provider = worker.provider_for(&model)?;
    let brief = plan_brief(store, &node).await?;
    tracing::info!(target: crate::trace::PROGRESS, "#{id} {} · plan by {}", node.title, short(&model));
    let answer = provider
        .complete(CompletionRequest {
            model: model.clone(),
            messages: vec![Message::system(PLAN_INSTRUCTION), Message::user(&brief)],
            temperature: Some(0.2),
            max_tokens: Some(4000),
            tools: Vec::new(),
            workspace: None,
            budget_usd: None,
            resume: None,
            max_turns: Some(1),
            effort: Some(Effort::default().to_string()),
        })
        .await
        .map_err(map_error)?;
    worker.record_as(&model, Some("planner"), Some(id), &answer.usage, answer.turns).await;
    let mut outline = ra_roadmap::outline::parse(&answer.text).context("the planner's answer is not an outline")?;
    // a planner that answered in headings meant cards: under a milestone they nest as sections,
    // under a section every item is a task
    outline.title = None;
    for item in &mut outline.items {
        item.kind = match (node.kind, item.kind) {
            (NodeKind::Milestone, NodeKind::Project | NodeKind::Milestone | NodeKind::Section) => NodeKind::Section,
            (NodeKind::Milestone, kind) => kind,
            (_, NodeKind::Subtask) => NodeKind::Subtask,
            _ => NodeKind::Task,
        };
    }
    Ok(Plan { id, title: node.title, model, outline, usage: answer.usage })
}

/// What the planner reads: the project context with the standing direction, the node with its
/// objective and acceptance, and the cards already under it with their states, so none comes
/// back twice.
async fn plan_brief(store: &Store, node: &Node) -> anyhow::Result<String> {
    let mut text = String::new();
    let context = project_direction(store).await?;
    if !context.trim().is_empty() {
        let _ = writeln!(text, "# Project context\n{}", context.trim_end());
    }
    let _ = writeln!(text, "\n# {} #{} {}", node.kind, node.id, node.title);
    if let Some(objective) = &node.objective {
        let _ = writeln!(text, "\nObjective: {objective}");
    }
    if let Some(acceptance) = &node.acceptance {
        let _ = writeln!(text, "\nAcceptance: {acceptance}");
    }
    let nodes = store.all().await?;
    let existing: Vec<&Node> = tree::subtree(&nodes, node.id).into_iter().filter(|n| n.id != node.id).collect();
    text.push_str("\n## Cards already under it\n");
    if existing.is_empty() {
        text.push_str("none yet\n");
    }
    for card in existing {
        let _ = writeln!(text, "- #{} {} {} [{}]", card.id, card.kind, card.title, card.state);
        if let Some(objective) = &card.objective {
            let _ = writeln!(text, "  objective: {objective}");
        }
    }
    Ok(text)
}

/// How much of the notes the digest role reads: enough for the report, not a long check output.
const DIGEST_NOTES: usize = 6000;

/// Writes the project's digest under the hub for the other projects' runs: the live run or walk
/// when one is named, the last end as `drafted` says it or as the paired feed does, the ready
/// count of the roadmap, and the files the last run changed, or the ones of the digest before
/// when this write has none.
async fn write_digest(store: &Store, root: &Path, live: Option<String>, changed: Option<Vec<String>>, drafted: Option<String>) {
    let nodes = store.all().await.unwrap_or_default();
    let ready = tree::effective_states(&nodes).values().filter(|s| **s == TaskState::Ready).count();
    // the files of the cards sent back for revision that are still dirty, so a later card
    // knows the tree it inherits; a clean tree names none
    let revising: Vec<i64> = nodes.iter().filter(|n| n.state == TaskState::NeedsRevision).map(|n| n.id).collect();
    let leftovers = ra_context::digest::leftovers(root, &revising)
        .into_iter()
        .map(|(path, ids)| format!("{path} ({})", ids.iter().map(|id| format!("#{id}")).collect::<Vec<_>>().join(" ")))
        .collect();
    let before = crate::hub::own_digest(root);
    let path = std::fs::canonicalize(root)
        .map_or_else(|_| root.display().to_string(), |p| p.display().to_string().trim_start_matches(r"\\?\").to_owned());
    crate::hub::write_digest(&crate::hub::Digest {
        name: crate::hub::project_name(root),
        path,
        pid: std::process::id(),
        when: unix_now(),
        live,
        last: drafted.or_else(|| crate::pair::last_end(root)),
        ready,
        changed: changed.or_else(|| before.map(|d| d.changed)).unwrap_or_default(),
        leftovers,
    });
}

/// A unix time as the clock of the day, `hh:mmZ`: UTC, which every project's run reads alike.
fn clock(when: u64) -> String {
    let day = when % 86_400;
    format!("{:02}:{:02}Z", day / 3600, day / 60 % 60)
}

/// Takes the hub's lock on the cargo target of `root` for the check, waiting on a check of
/// another project that builds in the same target; the wait is said once, on the progress
/// channel and the paired feed. `None` when the lock cannot be opened: the check goes unguarded.
async fn guard_target(root: &Path) -> Option<crate::hub::Guard> {
    let target = crate::hub::cargo_target(root);
    let name = crate::hub::target_lock_name(&target);
    let mut said = false;
    loop {
        match crate::hub::try_guard(&name) {
            Ok(Some(guard)) => return Some(guard),
            Ok(None) => {}
            Err(e) => {
                tracing::warn!("the check runs without the lock on {}: {e:#}", target.display());
                return None;
            }
        }
        if !said {
            said = true;
            let text = format!("wait: the check waits on the cargo target {}, which another project's check builds in", target.display());
            tracing::info!(target: crate::trace::PROGRESS, "  {text}");
            crate::pair::event(root, &text);
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

/// The share of the plan's five-hour window one run of a card is taken to use: the margin a run
/// keeps from the cap, and what a run live in another project counts for before the limits
/// show it.
const RUN_SHARE: f64 = 0.1;

/// How often a held run reads the limits again.
const PLAN_POLL: std::time::Duration = std::time::Duration::from_mins(1);

/// What one project's `var/limits.json` and digest say, for the hold before a run.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanReading {
    pub name: String,
    /// The five-hour window as the project last saw it, and when.
    pub five_hour: Option<ra_providers::limits::Window>,
    /// The week's window as the project last saw it.
    pub seven_day: Option<ra_providers::limits::Window>,
    pub seen_at: u64,
    /// Whether a run or a walk of the project is going right now.
    pub live: bool,
}

/// Holds the run while the plan's five-hour window, across every project on the machine, is
/// within one run of its cap: says so once, on the progress channel and the paired feed, and
/// reads the limits again every minute until there is room.
async fn wait_for_plan(root: &Path) {
    let this = crate::hub::project_name(root);
    let mut said = false;
    while let Some(text) = plan_hold(&plan_readings(root), &this, unix_now()) {
        if !said {
            said = true;
            tracing::info!(target: crate::trace::PROGRESS, "  {text}");
            crate::pair::event(root, &text);
        }
        tokio::time::sleep(PLAN_POLL).await;
    }
}

/// The plan readings of every project the machine remembers: the window's tabs, the projects
/// with a digest, and `root`, each once.
fn plan_readings(root: &Path) -> Vec<PlanReading> {
    let digests = crate::hub::digests();
    let mut projects: Vec<PathBuf> = crate::hub::recent();
    projects.extend(digests.iter().map(|d| PathBuf::from(&d.path)));
    projects.push(root.to_path_buf());
    let mut readings: Vec<PlanReading> = Vec::new();
    for project in projects {
        let name = crate::hub::project_name(&project);
        if readings.iter().any(|r| r.name == name) {
            continue;
        }
        let limits = ra_providers::limits::Limits::load(&project.join(ra_providers::claude_code::LIMITS_FILE));
        let live = digests.iter().any(|d| d.name == name && d.live.is_some() && crate::hub::pid_alive(d.pid));
        readings.push(PlanReading {
            name,
            five_hour: limits.windows.get("five_hour").copied(),
            seven_day: limits.windows.get(ra_providers::limits::WEEK).copied(),
            seen_at: limits.seen_at,
            live,
        });
    }
    readings
}

/// The project's threshold on the plan's week, from the `[plan]` table of its `rhipper.toml`;
/// [`ra_providers::limits::DEFAULT_THRESHOLD`] without one, or with one that does not read.
#[must_use]
pub fn plan_threshold(root: &Path) -> f64 {
    crate::router::Router::load(root).map_or(ra_providers::limits::DEFAULT_THRESHOLD, |r| r.plan.threshold())
}

/// The account's week and the project that last saw it, when it is at or past `threshold`: the
/// latest reading of a week that has not reset, whichever project took it. Room is `None`.
fn plan_week(readings: &[PlanReading], threshold: f64, now: u64) -> Option<(&str, ra_providers::limits::Window)> {
    readings
        .iter()
        .filter_map(|r| r.seven_day.filter(|w| w.resets_at > now).map(|w| (r.name.as_str(), r.seen_at, w)))
        .max_by_key(|(_, seen_at, _)| *seen_at)
        .filter(|(_, _, window)| window.utilization >= threshold)
        .map(|(name, _, window)| (name, window))
}

/// Why a run in `this` project is held, or `None` when the plan has room. The harness reports
/// the account's window whichever project asked, so the window's level is the latest reading
/// any project has of a window that has not reset; a run live in another project has used what
/// no reading shows yet, one [`RUN_SHARE`] each. The run is held while the level, those runs
/// and its own share together reach the cap, and the line names the projects behind it.
fn plan_hold(readings: &[PlanReading], this: &str, now: u64) -> Option<String> {
    let (name, window) = plan_window(readings, now)?;
    let live: Vec<&str> = readings.iter().filter(|r| r.live && r.name != this).map(|r| r.name.as_str()).collect();
    #[allow(clippy::cast_precision_loss)]
    let level = window.utilization + RUN_SHARE * live.len() as f64;
    if level + RUN_SHARE < 1.0 {
        return None;
    }
    let with = if live.is_empty() { String::new() } else { format!(", with a run live in {}", live.join(", ")) };
    Some(format!(
        "hold: the plan's five-hour window is at {:.0}% as {name} last saw it{with}; this run waits for room, the window resets in {}",
        window.utilization * 100.0,
        ra_providers::limits::until(window.resets_at)
    ))
}

/// The account's five-hour window and the project that last saw it: the latest reading of a
/// window that has not reset, whichever project took it.
fn plan_window(readings: &[PlanReading], now: u64) -> Option<(&str, ra_providers::limits::Window)> {
    readings
        .iter()
        .filter_map(|r| r.five_hour.filter(|w| w.resets_at > now).map(|w| (r.name.as_str(), r.seen_at, w)))
        .max_by_key(|(_, seen_at, _)| *seen_at)
        .map(|(name, _, window)| (name, window))
}

/// What the start line and the pair feed say of a tight pick.
const TIGHT_PICK: &str = "tight: smallest first";

/// Whether the window is tight by `readings`: less than [`ra_providers::TIGHT_SHARE`] of the
/// five-hour window is left. No reading is not tight.
fn plan_tight_in(readings: &[PlanReading], now: u64) -> bool {
    plan_window(readings, now).is_some_and(|(_, window)| 1.0 - window.utilization < ra_providers::TIGHT_SHARE)
}

/// Whether the plan's five-hour window is tight, read across every project the machine
/// remembers, as [`wait_for_plan`] reads it.
#[must_use]
pub fn plan_tight(root: &Path) -> bool {
    plan_tight_in(&plan_readings(root), unix_now())
}

/// Whether `limits`, one project's reading, say the window is tight: for the window, which
/// holds its project's reading already.
#[must_use]
pub fn limits_tight(limits: &ra_providers::limits::Limits) -> bool {
    let reading = PlanReading {
        name: String::new(),
        five_hour: limits.windows.get("five_hour").copied(),
        seven_day: limits.windows.get(ra_providers::limits::WEEK).copied(),
        seen_at: limits.seen_at,
        live: false,
    };
    plan_tight_in(&[reading], unix_now())
}

/// Runs the first ready card, the one `rc next` names; a line says so when none is.
///
/// # Errors
/// When the roadmap cannot be read, or the run fails.
pub async fn run_next(store: &Store, worker: &Worker) -> anyhow::Result<Vec<String>> {
    let roadmap = store.load().await?;
    match tree::next_ready(&roadmap.nodes, &roadmap.gambits) {
        Some(node) => run(store, node.id, worker).await,
        None => Ok(vec!["no card is ready".to_owned()]),
    }
}

/// Runs the ready cards one after another, reading readiness again after each, until none is
/// ready or a run fails; with `stop_on_revision` a card that needs revision stops the batch too.
/// When none is ready and an active gambit says `plan-next`, the planner fills the next empty
/// milestone first ([`crate::commands::run::plan_next`]) and the walk goes on with its cards.
/// Each run is its own bubble and its own handoff, through [`run`]. A failed run ends the batch
/// with its reason on the last line rather than as an error, so the runs before it keep their
/// lines.
///
/// # Errors
/// When the roadmap cannot be read between runs.
pub async fn run_all(store: &Store, worker: &Worker, stop_on_revision: bool) -> anyhow::Result<Vec<String>> {
    let mut lines = Vec::new();
    let mut ran = 0;
    // the card the last run continued and how many runs in a row it has had: a card the
    // project's turn cap keeps ending is picked again fresh, up to [`RESTARTS`] times
    let mut again: Option<(i64, usize)> = None;
    // whether the planner filled the roadmap since the last run: it plans once between runs,
    // so a plan that leaves no card ready ends the walk rather than plan the next milestone too
    let mut planned = false;
    // one walk per project: the lock is held for the whole of it
    let Some(_lock) = take_walk_lock(&workspace_of(store)) else {
        anyhow::bail!("a walk is already running in this project; `rc walk --stop` ends it after its current card")
    };
    // the other projects see the walk from its start
    let root = workspace_of(store);
    write_digest(store, &root, Some(format!("walk · pid {} · started {}", std::process::id(), clock(unix_now()))), None, None).await;
    loop {
        let roadmap = store.load().await?;
        // the walk's end, whichever way, goes to the paired session's feed and the digest
        let done = async |lines: &mut Vec<String>, text: String| {
            crate::pair::event(&root, &format!("walk {text}"));
            write_digest(store, &root, None, None, None).await;
            lines.push(text);
        };
        // the plan's room decides the pick before each card: a window that got tight during
        // the walk takes the small cards from here on
        let tight = plan_tight(&root);
        let Some(id) = batch_ready(&roadmap.nodes, &roadmap.gambits, tight).map(|n| n.id) else {
            // a `plan-next` gambit lets the planner fill the next empty milestone; its cards
            // are the next picks
            if !planned && crate::commands::run::plans_next(&roadmap.gambits) {
                planned = true;
                match crate::commands::run::plan_next(store, &root, worker).await {
                    Ok(Some(plan_lines)) => {
                        lines.extend(plan_lines);
                        continue;
                    }
                    Ok(None) => {
                        done(&mut lines, format!("ran {ran} card(s) · no card is ready and no milestone is left to plan")).await;
                        return Ok(lines);
                    }
                    Err(error) => {
                        done(&mut lines, format!("ran {ran} card(s) · no card is ready · planning the next milestone failed: {error:#}"))
                            .await;
                        return Ok(lines);
                    }
                }
            }
            done(&mut lines, format!("ran {ran} card(s) · no card is ready")).await;
            return Ok(lines);
        };
        let runs = match again {
            Some((last, runs)) if last == id => runs + 1,
            _ => 1,
        };
        again = Some((id, runs));
        if runs > RESTARTS {
            done(
                &mut lines,
                format!("ran {ran} card(s) · stopped: #{id} continued {RESTARTS} times in a row; raise [worker] turns or split the card"),
            )
            .await;
            return Ok(lines);
        }
        match run_picked(store, id, worker, tight).await {
            Ok(run_lines) => lines.extend(run_lines),
            Err(error) => {
                done(&mut lines, format!("ran {ran} card(s) · stopped: the run of #{id} failed: {error:#}")).await;
                return Ok(lines);
            }
        }
        ran += 1;
        planned = false;
        if stop_on_revision && store.read(id).await?.state == TaskState::NeedsRevision {
            done(&mut lines, format!("ran {ran} card(s) · stopped: #{id} needs revision")).await;
            return Ok(lines);
        }
        // `stop` from the console or the window: the walk ends after the run it was in
        let stop = root.join(WALK_STOP);
        if stop.exists() {
            let _ = std::fs::remove_file(&stop);
            done(&mut lines, format!("ran {ran} card(s) · stopped after #{id}, as asked")).await;
            return Ok(lines);
        }
    }
}

/// How many runs in a row a walk gives one card before it stops: a card the project's turn cap
/// ends this often is too large for the cap.
const RESTARTS: usize = 5;

/// The file a `stop` leaves for a walk, which reads it between runs.
const WALK_STOP: &str = "var/walk.stop";
/// The file `rc walk` leaves for the window, which starts the walk as its own process.
const WALK_START: &str = "var/walk.start";
/// The file a running walk holds open for exclusive use, so a second one does not start.
const WALK_LOCK: &str = "var/walk.lock";

/// The lock a running walk holds; dropping it lets the next walk start.
pub struct WalkLock(#[allow(dead_code)] std::fs::File);

/// Takes the project's walk lock: `None` while a walk runs there. On Windows the file is opened
/// with no sharing, which a second opener cannot do; elsewhere a pid file stands in.
#[must_use]
pub fn take_walk_lock(root: &Path) -> Option<WalkLock> {
    let _ = std::fs::create_dir_all(root.join("var"));
    let path = root.join(WALK_LOCK);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new().write(true).create(true).truncate(false).share_mode(0).open(&path).ok()?;
        Some(WalkLock(file))
    }
    #[cfg(not(windows))]
    {
        if let Ok(text) = std::fs::read_to_string(&path)
            && let Ok(pid) = text.trim().parse::<u32>()
            && pid != std::process::id()
            && Path::new(&format!("/proc/{pid}")).exists()
        {
            return None;
        }
        let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(&path).ok()?;
        use std::io::Write;
        let _ = write!(file, "{}", std::process::id());
        Some(WalkLock(file))
    }
}

/// Whether a walk is running in the project right now.
#[must_use]
pub fn walk_running(root: &Path) -> bool {
    take_walk_lock(root).is_none()
}

/// Asks the window that has the project to start the walk: leaves the file its tick reads.
#[must_use]
pub fn request_walk(root: &Path) -> String {
    let _ = std::fs::create_dir_all(root.join("var"));
    match std::fs::write(root.join(WALK_START), "") {
        Ok(()) => "the window starts the walk; its cards land in the project's tab and in var/pair.log".to_owned(),
        Err(e) => format!("cannot ask the window to start the walk: {e}"),
    }
}

/// Whether `rc walk` asked for a walk; the request is consumed.
#[must_use]
pub fn walk_requested(root: &Path) -> bool {
    let path = root.join(WALK_START);
    path.exists() && std::fs::remove_file(path).is_ok()
}

/// Starts the walk as a process of its own in `root`, detached from whoever asked: the
/// installed `rc run --all`, or the one beside this binary. Returns the process id.
///
/// # Errors
/// When a walk is running already, or `rc` cannot start.
pub fn spawn_walk(root: &Path, debug: bool) -> anyhow::Result<u32> {
    anyhow::ensure!(!walk_running(root), "a walk is already running in this project");
    let rc = std::env::current_exe()?.with_file_name(format!("rc{}", std::env::consts::EXE_SUFFIX));
    let mut command = std::process::Command::new(&rc);
    command.args(["run", "--all"]).current_dir(root);
    if !debug {
        command.arg("--no-debug");
    }
    crate::hub::detach(&mut command);
    let child = command.spawn().map_err(|e| anyhow::anyhow!("cannot start {}: {e}", rc.display()))?;
    Ok(child.id())
}

/// Asks the walk to stop after the task it is in: leaves the file it reads between runs. A
/// walk that is not running finds the file at its next start and stops after one card, so the
/// line says what to do about that.
pub fn stop_walk(root: &Path) -> String {
    let path = root.join(WALK_STOP);
    let _ = std::fs::create_dir_all(root.join("var"));
    match std::fs::write(&path, "") {
        Ok(()) => "the walk stops after the task it is in; `quit` is the word that closes the window".to_owned(),
        Err(e) => format!("cannot ask the walk to stop: {e}"),
    }
}

/// The order a walk would take the cards in, as `rc run --all` picks them one after another,
/// each pick counted as implemented for the next: the projected order of the work, for the
/// window's walk view. With `tight` the picks are the smallest ready card first, as a walk picks
/// when the plan's window is tight ([`plan_tight`]). Nothing is written.
#[must_use]
pub fn walk_order(nodes: &[Node], gambits: &[Gambit], tight: bool) -> Vec<i64> {
    let mut nodes = nodes.to_vec();
    let mut order = Vec::new();
    while let Some(id) = batch_ready(&nodes, gambits, tight).map(|n| n.id) {
        order.push(id);
        if let Some(node) = nodes.iter_mut().find(|n| n.id == id) {
            node.state = TaskState::Implemented;
        }
    }
    order
}

/// The first card a batch runs: a defined task whose waits-on are each accepted or implemented,
/// scored as `rc next` scores ([`tree::rank`]): priority, then the gambits, then tree order.
/// Implemented counts beside accepted here, unlike in [`tree::next_ready`]: the cards the batch
/// implements are not accepted until the person reviews, and their dependents are what the batch
/// is meant to run ahead of that review. A card whose waits-on failed, left the roadmap or are
/// still open is not picked, so every pick moves a card out of defined and the loop ends; a
/// card still in progress is picked too, since a walk holds the project and such a card is one
/// a run left to continue, as the project's turn cap does. With `tight` the pick is the ready
/// card of the lowest complexity, the best scored among equals: what is left of the window
/// finishes small cards whole rather than starting a large one it cannot end.
fn batch_ready<'a>(nodes: &'a [Node], gambits: &'a [Gambit], tight: bool) -> Option<&'a Node> {
    ready_cards(nodes, gambits, tight).into_iter().next()
}

/// Every card a batch may run, in the order [`batch_ready`] would pick them: by score, or by
/// complexity with `tight`, the smallest first.
fn ready_cards<'a>(nodes: &'a [Node], gambits: &'a [Gambit], tight: bool) -> Vec<&'a Node> {
    let met = |dep: &i64| nodes.iter().any(|d| d.id == *dep && matches!(d.state, TaskState::Accepted | TaskState::Implemented));
    let candidates = nodes.iter().filter(|n| {
        matches!(n.state, TaskState::Defined | TaskState::InProgress)
            && matches!(n.kind, NodeKind::Task | NodeKind::Subtask)
            && n.depends_on.iter().all(met)
    });
    let mut ready: Vec<&Node> = tree::rank(nodes, candidates, gambits).into_iter().map(|s| s.node).collect();
    if tight {
        ready.sort_by_key(|n| n.complexity);
    }
    ready
}

/// Where the worktrees of the cards live, under the project's `var/`.
const WORKTREES_DIR: &str = "var/worktrees";

/// The branch a card's worktree is on.
#[must_use]
pub fn worktree_branch(id: i64) -> String {
    format!("card/{id}")
}

/// The worktree of card `id` in the project at `root`: `var/worktrees/<id>` on the branch
/// `card/<id>`, made from HEAD when it is not there yet; a second run of the card reuses the
/// worktree and the branch as they stand. The path and the branch.
///
/// # Errors
/// When the project is not a git repository, or git cannot add the worktree.
pub async fn worktree_add(root: &Path, id: i64) -> anyhow::Result<(PathBuf, String)> {
    let path = root.join(WORKTREES_DIR).join(id.to_string());
    let branch = worktree_branch(id);
    if path.join(".git").exists() {
        return Ok((path, branch));
    }
    std::fs::create_dir_all(root.join(WORKTREES_DIR)).with_context(|| format!("cannot create {WORKTREES_DIR}"))?;
    // the branch outlives a dropped worktree: a new worktree takes it up where it was
    let exists = git(root, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")]).await.is_ok();
    let target = path.to_string_lossy().into_owned();
    if exists {
        git(root, &["worktree", "add", &target, &branch]).await?;
    } else {
        git(root, &["worktree", "add", "-b", &branch, &target, "HEAD"]).await?;
    }
    Ok((path, branch))
}

/// The worktrees of the cards, one line each: the id, the branch and the path; a line says so
/// when there are none.
///
/// # Errors
/// When `var/worktrees` cannot be listed.
pub fn worktree_list(root: &Path) -> anyhow::Result<Vec<String>> {
    let dir = root.join(WORKTREES_DIR);
    if !dir.exists() {
        return Ok(vec!["no worktree".to_owned()]);
    }
    let mut ids: Vec<i64> = std::fs::read_dir(&dir)
        .with_context(|| format!("cannot list {}", dir.display()))?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().join(".git").exists())
        .filter_map(|entry| entry.file_name().to_string_lossy().parse().ok())
        .collect();
    ids.sort_unstable();
    if ids.is_empty() {
        return Ok(vec!["no worktree".to_owned()]);
    }
    Ok(ids.into_iter().map(|id| format!("#{id} · {} · {}", worktree_branch(id), dir.join(id.to_string()).display())).collect())
}

/// Removes the worktree of card `id`, changes and all, and its branch when it is merged; an
/// unmerged branch stays, and the line says so, as merging is the person's.
///
/// # Errors
/// When there is no such worktree, or git cannot remove it.
pub async fn worktree_drop(root: &Path, id: i64) -> anyhow::Result<Vec<String>> {
    let path = root.join(WORKTREES_DIR).join(id.to_string());
    anyhow::ensure!(path.join(".git").exists(), "#{id} has no worktree; `rc worktree list` names them");
    git(root, &["worktree", "remove", "--force", &path.to_string_lossy()]).await?;
    let branch = worktree_branch(id);
    let mut lines = vec![format!("dropped the worktree of #{id} at {}", path.display())];
    match git(root, &["branch", "-d", &branch]).await {
        Ok(_) => lines.push(format!("deleted {branch}: it was merged")),
        Err(_) => lines.push(format!("kept {branch}: it is not merged; `git merge {branch}` takes it, `git branch -D {branch}` drops it")),
    }
    Ok(lines)
}

/// One git command in `dir`, its stdout trimmed; the stderr is the error when it fails.
async fn git(dir: &Path, args: &[&str]) -> anyhow::Result<String> {
    let out = tokio::process::Command::new("git").args(args).current_dir(dir).output().await.context("cannot run git")?;
    anyhow::ensure!(out.status.success(), "git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// [`run`], in the card's worktree: the worker acts and the checks run in `var/worktrees/<id>`
/// on the branch `card/<id>`, which the check line and the handoff name; the roadmap and the
/// context stay the project's.
///
/// # Errors
/// When the worktree cannot be made, or as [`run`].
pub async fn run_in_worktree(store: &Store, id: i64, worker: &Worker) -> anyhow::Result<Vec<String>> {
    let (path, branch) = worktree_add(&workspace_of(store), id).await?;
    run(store, id, &worker.clone().with_worktree(path, branch)).await
}

/// Runs the ready cards `width` at a time, each in its own worktree, in waves: the ready cards
/// are read, up to `width` of them run together, and readiness is read again once the wave
/// ends, until no card is ready, a run fails, or with `stop_on_revision` a card needs revision.
/// A dependent of a card the batch implemented runs from HEAD like the rest: its dependency's
/// changes are on a branch until the person merges them.
///
/// # Errors
/// When the roadmap cannot be read between waves, or a worktree cannot be made.
pub async fn run_parallel(store: &Store, worker: &Worker, width: usize, stop_on_revision: bool) -> anyhow::Result<Vec<String>> {
    anyhow::ensure!(width > 0, "the width of a parallel batch is at least one");
    let root = workspace_of(store);
    let Some(_lock) = take_walk_lock(&root) else {
        anyhow::bail!("a walk is already running in this project; `rc walk --stop` ends it after its current card")
    };
    let mut lines = Vec::new();
    let mut ran = 0;
    loop {
        let roadmap = store.load().await?;
        let nodes = roadmap.nodes;
        let ids: Vec<i64> = ready_cards(&nodes, &roadmap.gambits, plan_tight(&root)).iter().take(width).map(|n| n.id).collect();
        if ids.is_empty() {
            lines.push(format!("ran {ran} card(s) · no card is ready"));
            return Ok(lines);
        }
        let mut runs: Vec<(i64, Worker)> = Vec::with_capacity(ids.len());
        for id in &ids {
            let (path, branch) = worktree_add(&root, *id).await?;
            runs.push((*id, worker.clone().with_worktree(path, branch)));
        }
        let wave = join_all(runs.iter().map(|(id, worker)| run(store, *id, worker))).await;
        let mut stopped = None;
        for (id, outcome) in ids.iter().zip(wave) {
            match outcome {
                Ok(run_lines) => {
                    lines.extend(run_lines);
                    ran += 1;
                    if stop_on_revision && stopped.is_none() && store.read(*id).await?.state == TaskState::NeedsRevision {
                        stopped = Some(format!("stopped: #{id} needs revision"));
                    }
                }
                Err(error) => {
                    stopped.get_or_insert(format!("stopped: the run of #{id} failed: {error:#}"));
                }
            }
        }
        if let Some(why) = stopped {
            lines.push(format!("ran {ran} card(s) · {why}"));
            return Ok(lines);
        }
    }
}

/// Drives `futures` together on the one task and collects their outputs in their order. The
/// runs of a wave overlap in their waits, the model and the checks, not in their bookkeeping:
/// the store reads and writes `rhipper.md` without yielding, so two runs never interleave a
/// read-modify-write of it, which a spawned task per run could not promise.
async fn join_all<F: Future>(futures: impl IntoIterator<Item = F>) -> Vec<F::Output> {
    let mut pending: Vec<Option<std::pin::Pin<Box<F>>>> = futures.into_iter().map(|f| Some(Box::pin(f))).collect();
    let mut outputs: Vec<Option<F::Output>> = (0..pending.len()).map(|_| None).collect();
    std::future::poll_fn(|cx| {
        for (slot, out) in pending.iter_mut().zip(outputs.iter_mut()) {
            if let Some(future) = slot
                && let std::task::Poll::Ready(value) = future.as_mut().poll(cx)
            {
                *out = Some(value);
                *slot = None;
            }
        }
        if pending.iter().all(Option::is_none) { std::task::Poll::Ready(()) } else { std::task::Poll::Pending }
    })
    .await;
    outputs.into_iter().flatten().collect()
}

/// The work of [`run`], with the cost of the session and the count of the gotchas it hit beside
/// the log lines.
async fn run_task(store: &Store, node: Node, worker: &Worker, roles: &Roles) -> anyhow::Result<(Vec<String>, Option<f64>, usize)> {
    let id = node.id;
    let provider = worker.provider()?;
    store.set_state_by(id, TaskState::InProgress, Source::Run).await?;

    let Prepared { workspace, root, mut task, project, budget, caps, effort } = prepare(store, &node, worker, true).await?;
    // a fix pass reads the verdict first: the points are the task, the card is the setting
    if let Some(verdict) = &worker.fix {
        task.brief = format!("{FIX_INSTRUCTION}\n{verdict}\n\n{}", task.brief);
    }
    let workspace_dir = workspace.clone();
    // where the changes land: the worktree when the run has one, else the project
    let acting = workspace_dir.clone().unwrap_or_else(|| root.clone());
    // what is dirty before the run, so the handoff names only what the run changed
    let dirty_before = git_status(&acting).await;
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
        // harness holds it to the budget; its hooks learn the card from the environment
        begin_run(&acting, id);
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
                max_turns: caps.limit(),
                effort: Some(effort.to_string()),
            })
            .await;
        match answer {
            Ok(answer) => {
                worker.record(Some(id), &answer.usage, answer.turns).await;
                gotcha_hits.clone_from(&answer.gotchas);
                if let Some(session) = answer.session_id.as_deref() {
                    // kept on the node with the card's fingerprint: the next run of it resumes
                    // this session while the card reads the same
                    store.set_session(id, Some((session, &card_fingerprint(&node))), unix_now()).await?;
                }
                Ok(Session {
                    text: answer.text,
                    usage: answer.usage,
                    calls: 0,
                    turns: answer.turns,
                    turn_usage: answer.turn_usage,
                    cap: caps.limit(),
                    capped: None,
                    reads: answer.reads,
                })
            }
            // the harness reports its turns running out as an error, with what the run read,
            // wrote and cost: that is booked like a finished run's, so the card's cost counts
            // every session. When the project's cap is what ran out, the run ended as meant and
            // the card continues fresh
            Err(ProviderError::TurnsOut(answer)) => {
                worker.record(Some(id), &answer.usage, answer.turns).await;
                gotcha_hits.clone_from(&answer.gotchas);
                match caps.restarts_at() {
                    Some(restart) => Ok(Session {
                        text: answer.text,
                        usage: answer.usage,
                        calls: 0,
                        turns: u64::from(restart),
                        turn_usage: answer.turn_usage,
                        cap: Some(restart),
                        capped: Some(restart),
                        reads: answer.reads,
                    }),
                    None => Err(map_error(ProviderError::TurnsOut(answer))),
                }
            }
            Err(error) => Err(map_error(error)),
        }
    } else {
        tool_loop(provider.as_ref(), worker, id, bare_context(&task, &project, worker.debug), workspace.as_deref(), budget, caps).await
    };
    // the handoff of this run, for the next one of the card; the done cards' files go
    let handoff = |outcome: &str, check: &str, report: &str, changed: Vec<String>| ra_context::Handoff {
        when: unix_now(),
        model: worker.model.clone(),
        outcome: outcome.to_owned(),
        check: check.to_owned(),
        verdict: None,
        changed,
        report: report.to_owned(),
        warnings: Vec::new(),
    };
    let mut session = match session {
        Ok(session) => session,
        Err(error) => {
            // a spent budget is worth a note: the next attempt sees what the last one cost
            if let Some(over) = error.downcast_ref::<OverBudget>() {
                store.set_notes(id, &over.to_string()).await?;
            }
            store.set_state_by(id, TaskState::Failed, Source::Run).await?;
            let changed = ra_context::changed_between(&dirty_before, &git_status(&acting).await);
            task.record(handoff("failed", "no check: the run failed", &format!("{error:#}"), changed));
            save_context(store, &root, &task).await;
            return Err(error);
        }
    };
    // a word for another card: the `notes-for #<id>:` lines go to that card's context as a
    // handoff from this one, and out of this card's own notes and handoff
    let (text, notes_for) = ra_context::notes_for(&session.text);
    session.text = text;
    let mut noted = Vec::new();
    for (to, text) in notes_for {
        if to == id || store.read(to).await.is_err() {
            noted.push(format!("  notes-for #{to} dropped: no such card"));
        } else if let Err(e) = ra_context::TaskContext::leave_note(&root, to, id, &worker.model, unix_now(), &text) {
            noted.push(format!("  notes-for #{to} not left: {e:#}"));
        } else {
            noted.push(format!("  notes-for #{to}: left on its context"));
        }
    }
    // the project's turn cap ended the run: no check, no verdict, the card stays in progress
    // with a handoff of what was done and what is left, and its session is dropped so the next
    // run, which the walk starts right away, opens on a small context again
    if let Some(cap) = session.capped {
        store.set_session(id, None, unix_now()).await?;
        let changed = ra_context::changed_between(&dirty_before, &git_status(&acting).await);
        let report = capped_report(cap, &session, &changed, &node);
        store.set_notes(id, &report).await?;
        let why = format!("the turn cap of {cap} ended the run; the card continues in a fresh session");
        let mut last = handoff("continues", &format!("no check: {why}"), &report, changed);
        if let Some(branch) = &worker.branch {
            last.check = format!("{} · branch {branch}", last.check);
        }
        task.record(last);
        save_context(store, &root, &task).await;
        let sizes = turn_sizes(&session);
        let mut lines = vec![format!(
            "↻ #{id} {} continues on {} · turns {}/{cap}, {} tool call(s) · in {} out {} · {sizes} · {} · {why}",
            node.title,
            short(&worker.model),
            session.turns,
            session.calls,
            session.usage.prompt_tokens,
            session.usage.completion_tokens,
            cost_text(session.usage.cost_usd)
        )];
        lines.extend(fresh);
        lines.extend(noted);
        return Ok((lines, session.usage.cost_usd, gotcha_hits.len()));
    }
    // the local checks before the card counts as done: no model call, the acceptance names
    // them (else the project, else the workspace tests), and they run in order up to the first
    // failure
    let Checks { commands, lint: project_lint } = checks_of(&root, &node);
    let command = commands.join(" · ");
    let toolbox = worker.toolbox(workspace_dir.as_deref().unwrap_or(Path::new(".")));
    // a cargo check builds in a target another project may share: the hub's lock on it, so
    // two builds do not fight over the directory
    let _target = if commands.iter().any(|c| c.trim_start().starts_with("cargo")) { guard_target(&root).await } else { None };
    let checks = run_checks(&toolbox, &commands).await;
    let changed = ra_context::changed_between(&dirty_before, &git_status(&acting).await);
    // a failed check leads the notes and the handoff: the failed command with its tests and the
    // tail of its output first, then the commands that passed before it with their exit codes,
    // then the worker's answer, so the next run reads the failures before anything
    let mut report = session.text.clone();
    // what leads the answer in the notes and the handoff alike: the failed check, when one did
    let mut lead = String::new();
    let (mut passed, mut check_line) = match checks.iter().find(|run| !run.passed) {
        None => (true, format!("  check passed: {command}")),
        Some(failed) => {
            let before: Vec<String> = checks
                .iter()
                .filter(|run| run.passed)
                .map(|run| format!("check passed: {} · {}\n{}", run.command, run.exit, run.text))
                .collect();
            let before = if before.is_empty() { String::new() } else { format!("{}\n\n", before.join("\n\n")) };
            lead = format!("check failed: {}\n{}\n\n{before}", failed.command, failed.text);
            report = format!("{lead}{}", session.text);
            let count = match failed.failed_tests {
                0 => String::new(),
                n => format!(" · {n} test(s) failed"),
            };
            let after = match checks.len() {
                1 => String::new(),
                n => format!(" · after {} of {} passed", n - 1, commands.len()),
            };
            let where_ = if failed.exit.starts_with("exit code") || failed.exit.starts_with("stopped") {
                " · the output is in the notes".to_owned()
            } else {
                format!(" · {}", failed.exit)
            };
            (false, format!("  check failed: {}{count}{after}{where_}", failed.command))
        }
    };
    // the lint of what the run changed, once the check passed: the orchestrator finds the
    // warnings so the worker spends no turns on them. A lint that cannot run is a line, not a
    // verdict; a warning in a changed file is.
    let mut lint_lines = Vec::new();
    let mut lint_warnings = Vec::new();
    if passed && let Ok(toolbox) = &toolbox {
        let lint = lint_changed(toolbox, &acting, &changed, project_lint.as_deref()).await;
        for skipped in &lint.skipped {
            lint_lines.push(format!("  lint skipped: {skipped}"));
        }
        if lint.warnings.is_empty() {
            if !lint.command.is_empty() {
                lint_lines.push(format!("  lint clean: {}", lint.command));
            }
        } else {
            passed = false;
            report = format!("{}\n\n{}", lint.warnings.join("\n\n"), session.text);
            check_line = format!(
                "  check passed: {command} · lint: {} warning(s) in the changed files from {} · the lines are in the notes",
                lint.warnings.len(),
                lint.command
            );
            lint_warnings = lint.warnings;
        }
    }
    if passed {
        store.set_notes(id, &session.text).await?;
        store.set_state_by(id, TaskState::Implemented, Source::Run).await?;
    } else {
        if report != session.text {
            store.set_notes(id, &report).await?;
        }
        store.set_state_by(id, TaskState::NeedsRevision, Source::Run).await?;
    }
    // the run against its estimate, on the check line so the handoff carries it: the next run
    // reads what the last one spent against what the card was sized at
    let verdict = estimate_verdict(estimate_of(&node), average_turn(&session));
    let sizes = turn_sizes(&session);
    check_line = format!("{check_line} · {verdict} · {sizes}");
    // what to do with the build, on the same line: the window restarts on the milestone commit,
    // not per card
    if let Some(advice) = build_advice(&changed) {
        check_line = format!("{check_line} · {advice}");
    }
    // the branch the changes sit on, when the run had a worktree: merging it is the person's
    if let Some(branch) = &worker.branch {
        check_line = format!("{check_line} · branch {branch}");
    }
    // the summary role condenses the answer for the handoff, when it has a model; the notes of
    // the card keep the answer whole either way. The warnings ride in their own section of the
    // handoff, not at the head of its report: the notes lead with them, the next run gets them
    // in its machine notes
    let summary = worker.ask_role(roles, Role::Summary, Some(id), SUMMARY_INSTRUCTION, &session.text).await;
    let answer = summary.as_deref().unwrap_or(&session.text);
    let handed = if lint_warnings.is_empty() { format!("{lead}{answer}") } else { answer.to_owned() };
    let mut last = handoff(if passed { "implemented" } else { "needs revision" }, check_line.trim(), &handed, changed);
    last.warnings = lint_warnings;
    task.record(last);
    save_context(store, &root, &task).await;
    let turns = match session.cap {
        Some(cap) => format!("{}/{cap}", session.turns),
        None => session.turns.to_string(),
    };
    let mut lines = vec![
        format!(
            "{} #{} {} {} on {} · turns {turns}, {} tool call(s) · in {} out {} · {sizes} · {} · {verdict}",
            if passed { "✓" } else { "✗" },
            id,
            node.title,
            if passed { "implemented" } else { "needs revision" },
            short(&worker.model),
            session.calls,
            session.usage.prompt_tokens,
            session.usage.completion_tokens,
            cost_text(session.usage.cost_usd)
        ),
        check_line,
    ];
    lines.extend(fresh);
    lines.extend(noted);
    lines.extend(lint_lines);
    // in debug mode the files the run read again, the context every later turn carried, and
    // the worker's own look back: kept for the next runs, and named here
    if worker.debug {
        if let Some(again) = reread(&session.reads) {
            lines.push(format!("  re-read: {again}"));
        }
        let mut notes = crate::retro::parse(&session.text);
        // a bullet without its tag is put to the classify role, when it has a model: the
        // worker forgets the tag more often than it misplaces it
        for note in notes.iter_mut().filter(|note| !note.machine) {
            if let Some(answer) = worker.ask_role(roles, Role::Classify, Some(id), crate::retro::CLASSIFY, &note.text).await
                && let Some(machine) = crate::retro::classified(&answer)
            {
                note.machine = machine;
            }
        }
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
    caps: Caps,
    /// The effort the run asks of the model: `--effort` when it was passed, else the card's.
    effort: Effort,
}

/// The turn caps of a run; nothing caps a run by default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Caps {
    /// The hard cap, when `--turns` named one: a run that reaches it failed.
    hard: Option<u32>,
    /// The project's `[worker] turns`, when it names one: the run ends there with a handoff and
    /// the card continues in a fresh session.
    restart: Option<u32>,
}

impl Caps {
    /// The turns the provider is told, the lower of the two: the one that ends a run first.
    fn limit(self) -> Option<u32> {
        match (self.hard, self.restart) {
            (Some(hard), Some(restart)) => Some(hard.min(restart)),
            (hard, restart) => hard.or(restart),
        }
    }

    /// The restart cap, when it is the one that ends the run first.
    fn restarts_at(self) -> Option<u32> {
        self.restart.filter(|r| self.hard.is_none_or(|hard| *r < hard))
    }
}

/// Builds what a run of `node` sends. The context of the task is the brief of the card, the
/// outlines of its files, and the handoffs of its earlier runs, so the worker reads regions and
/// continues rather than exploring again. With `draft`, a missing project context file is
/// written first, as a run does; a dry run reads what is there.
async fn prepare(store: &Store, node: &Node, worker: &Worker, draft: bool) -> anyhow::Result<Prepared> {
    // the worker acts in the directory of the roadmap file, the project it describes, unless a
    // worktree of the card was made for it; the context and `var/` are the project's either way
    let project =
        store.path().parent().filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf).or_else(|| std::env::current_dir().ok());
    let root = project.clone().unwrap_or_else(|| PathBuf::from("."));
    let workspace = worker.workspace.clone().or(project);
    let mut task = ra_context::TaskContext::open(&root, node.id);
    let brief = context_of(store, node).await?;
    task.brief = brief.text;
    task.outlines = ra_context::outline_files(&root, &brief.files, &last_change(&root, node, &task).await);
    let project =
        if draft { ra_context::Project::ensure(&root).unwrap_or_default() } else { ra_context::Project::load(&root).unwrap_or_default() };
    // the active gambits ride after the project context as the person's standing direction
    let project = ra_context::with_direction(&project, &direction_lines(&store.gambits().await?));
    // an explicit budget, else the complexity's while budgets are on, else none at all; a turn
    // cap only when one was asked for. No limit stands by default: the person asked for none
    let budget = worker.budget.or_else(|| worker.budgets.then(|| budget_for(node.complexity)));
    let caps = Caps { hard: worker.turns, restart: crate::router::Router::load(&root).ok().and_then(|router| router.worker.turns()) };
    let effort = worker.effort.unwrap_or(node.effort);
    Ok(Prepared { workspace, root, task, project, budget, caps, effort })
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
    let notes = [machine_notes(root), warnings_note(task), digest_block(&crate::hub::digest_lines(root))]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    if !notes.is_empty() {
        messages.push(Message::system(notes));
    }
    if debug {
        messages.push(Message::system(crate::retro::PROMPT));
    }
    messages.push(Message::user(task.prompt()));
    messages
}

/// The other projects' digests as a block of the system prompt, one line each, so the worker
/// knows what they are touching (a shared crate, the install dir); empty when there is none.
fn digest_block(lines: &[String]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = lines.iter().map(|l| format!("- {l}")).collect();
    format!("What the other projects on this machine are doing, one line each:\n{}", lines.join("\n"))
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
    let worker = &worker.for_card(&workspace_of(store), &node)?;
    let provider = worker.provider()?;
    let Prepared { workspace, root, task, project, budget, caps, effort } = prepare(store, &node, worker, false).await?;
    let harness = provider.has_own_tools();
    let checks = checks_of(&root, &node);
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
        max_turns: caps.limit(),
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
            "  budget {} · turn cap {}{} · resume {} · {}",
            budget.map_or_else(|| "none".to_owned(), |b| format!("${b:.2}")),
            caps.hard.map_or_else(|| "none".to_owned(), |c| c.to_string()),
            caps.restart.map_or_else(String::new, |r| format!(" · fresh session after {r} turns")),
            match resume_of(&node, unix_now()) {
                Ok(Some(session)) => session,
                Ok(None) => "none".to_owned(),
                Err(reason) => format!("none, {reason}"),
            },
            checks.describe()
        ),
    ];
    // what the pre hook makes of a check when the worker runs it itself: the command as it
    // runs, with its tail, shown before a run does it
    if let Ok(catalogue) = Catalogue::load(&root.join(GOTCHAS_FILE)) {
        for check in &checks.commands {
            if let ra_gotchas::Verdict::Rewrite { command, .. } = catalogue.command(check, ra_gotchas::Conditions::default()) {
                lines.push(format!("  the pre hook runs `{check}` as `{command}`"));
            }
        }
    }
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
    /// The command that linted: `cargo clippy -p` over the changed crates, or the project's own;
    /// empty when nothing was linted.
    command: String,
    /// The diagnostics pointing at a changed file, whole.
    warnings: Vec<String>,
    /// The lints that could not run, each with its reason.
    skipped: Vec<String>,
}

/// Lints what `changed` touched and keeps the diagnostics that point into the lines the change
/// touched (`git diff -U0` of those files; a file git does not know is new, and all of it
/// counts), not the warnings its files had before. Without a `project` lint, clippy runs over
/// each crate that owns a changed file; an empty one is no lint at all; any other is the
/// project's command, run as a check is, outside the shell allowlist. Clippy exits 0 on a
/// warning, so the output decides, not the exit code.
async fn lint_changed(toolbox: &ra_tools::Toolbox, root: &Path, changed: &[String], project: Option<&str>) -> Lint {
    let command = match project {
        Some("") => return Lint::default(),
        Some(command) if !changed.is_empty() => command.to_owned(),
        Some(_) => return Lint::default(),
        None => {
            let mut crates: Vec<String> = Vec::new();
            for file in changed {
                if let Some(krate) = ra_tools::crate_of(root, file)
                    && !crates.contains(&krate)
                {
                    crates.push(krate);
                }
            }
            if crates.is_empty() {
                return Lint::default();
            }
            format!("cargo clippy -p {}", crates.join(" -p "))
        }
    };
    let mut lint = Lint { command, ..Lint::default() };
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
    // one clippy over every crate: a crate's warnings come out again under each crate that
    // depends on it, so a run per crate counted the same warning three times (#92 read 68)
    let outcome = if project.is_some() { toolbox.check_acceptance(&lint.command).await } else { toolbox.check(&lint.command).await };
    match outcome {
        Ok((_, output)) => {
            for warning in ra_tools::warnings_within(&output, &ranges) {
                if !lint.warnings.contains(&warning) {
                    lint.warnings.push(warning);
                }
            }
        }
        Err(e) => lint.skipped.push(format!("{} · {e:#}", lint.command)),
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
    let brief = context_of(store, &node).await?;
    task.brief = brief.text;
    task.outlines = ra_context::outline_files(&root, &brief.files, &last_change(&root, &node, &task).await);
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

/// The person's standing direction as a run, the planner and a reassessment read it: one line per
/// active gambit, in the order of the file, with its weight and its focus words; the parked and
/// the done ones stay out.
#[must_use]
pub fn direction_lines(gambits: &[Gambit]) -> Vec<String> {
    gambits
        .iter()
        .filter(|g| g.state == GambitState::Active)
        .map(|g| {
            let mut line = format!("g{} {} {{w{}}}", g.id, g.text, g.weight);
            if !g.focus.is_empty() {
                let _ = write!(line, " · focus: {}", g.focus.join(" "));
            }
            line
        })
        .collect()
}

/// The project context with the standing direction after it, for the prompts that plan and
/// reassess; empty while neither is there.
///
/// # Errors
/// When the roadmap cannot be read.
pub async fn project_direction(store: &Store) -> anyhow::Result<String> {
    Ok(ra_context::with_direction(&project_context(store), &direction_lines(&store.gambits().await?)))
}

/// The workspace: the directory of the roadmap file.
pub fn workspace_of(store: &Store) -> PathBuf {
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

/// The check of a node when its acceptance names none.
const DEFAULT_CHECK: &str = "cargo test --workspace -q";

/// The checks of a node: every backticked span of its acceptance that reads as a command line,
/// in the order written, else the tests of the whole workspace. A span is a command when its
/// first token is a build or script runner (cargo, python, npm, gradle and the like) or a
/// relative script path (`./gw.sh`, `.\gw.ps1`, `tools/run.py`); a bare path, an identifier or a
/// `git log` style reference is not.
#[must_use]
pub fn check_commands(acceptance: Option<&str>) -> Vec<String> {
    check_commands_or(acceptance, DEFAULT_CHECK)
}

/// [`check_commands`] with `default` in place of the workspace tests.
fn check_commands_or(acceptance: Option<&str>, default: &str) -> Vec<String> {
    let commands: Vec<String> = acceptance
        .map(|text| text.split('`').skip(1).step_by(2).map(str::trim).filter(|span| is_command(span)).map(str::to_owned).collect())
        .unwrap_or_default();
    if commands.is_empty() { vec![default.to_owned()] } else { commands }
}

/// What a card is judged by in its project: the checks and the lint.
struct Checks {
    /// The card's own backticked commands, else the project's `check`, else the workspace tests.
    commands: Vec<String>,
    /// The project's `lint` as the `[worker]` table has it; `None` leaves clippy in place.
    lint: Option<String>,
}

impl Checks {
    /// One line naming both, for the dry run header, the activity log and the pair log.
    fn describe(&self) -> String {
        let lint = match self.lint.as_deref() {
            None => "clippy over the changed crates",
            Some("") => "none",
            Some(command) => command,
        };
        format!("check: {} · lint: {lint}", self.commands.join(" · then "))
    }
}

/// The checks of `node` under the `[worker]` table of the project at `root`; a table that will
/// not parse is the default one, as the run already said so when it picked the model.
fn checks_of(root: &Path, node: &Node) -> Checks {
    let worker = crate::router::Router::load(root).map(|router| router.worker).unwrap_or_default();
    Checks {
        commands: check_commands_or(node.acceptance.as_deref(), worker.check().unwrap_or(DEFAULT_CHECK)),
        lint: worker.lint().map(str::to_owned),
    }
}

/// The programs an acceptance runs a check through. `git` is not one: a backticked `git log`
/// is a reference, and the tools run the read-only git commands by themselves.
const CHECK_PROGRAMS: [&str; 34] = [
    "cargo", "rustc", "python", "python3", "py", "pytest", "uv", "pip", "node", "npm", "npx", "pnpm", "yarn", "bun", "deno", "tsc", "go",
    "make", "cmake", "ctest", "ninja", "just", "gradle", "gradlew", "mvn", "dotnet", "java", "zig", "swift", "ruby", "bundle", "php",
    "composer", "bash",
];

/// The extensions of a script named by a relative path.
const SCRIPT_EXTENSIONS: [&str; 5] = ["ps1", "sh", "py", "bat", "cmd"];

/// Whether a backticked span is a command line: one line whose first token is a check program
/// or a relative script path.
fn is_command(span: &str) -> bool {
    if span.lines().count() != 1 {
        return false;
    }
    let Some(first) = span.split_whitespace().next() else { return false };
    if CHECK_PROGRAMS.contains(&first) {
        return true;
    }
    let is_script = Path::new(first).extension().and_then(|e| e.to_str()).is_some_and(|e| SCRIPT_EXTENSIONS.contains(&e));
    let is_relative = !first.starts_with('/') && !first.starts_with('\\') && !first.contains(':');
    is_script && is_relative
}

/// What one command of a check came to, for the notes and the activity log.
struct CheckRun {
    command: String,
    passed: bool,
    /// The last line of the output: `exit code N`, or why the command did not run.
    exit: String,
    /// The output as the notes keep it: the failures and the tail of a failed command, the
    /// short tail of a passed one.
    text: String,
    failed_tests: usize,
}

/// How many lines of a passed command's output the notes keep.
const PASSED_TAIL_LINES: usize = 5;

/// Runs the checks of a card in the project directory, in the order written, and stops at the
/// first failure; each command's verdict goes to the activity log as it comes.
async fn run_checks(toolbox: &anyhow::Result<ra_tools::Toolbox>, commands: &[String]) -> Vec<CheckRun> {
    let mut runs = Vec::new();
    for command in commands {
        let outcome = match toolbox {
            Ok(toolbox) => toolbox.check_acceptance(command).await,
            Err(e) => Err(anyhow::anyhow!("{e:#}")),
        };
        let run = match outcome {
            Ok((true, output)) => CheckRun {
                command: command.clone(),
                passed: true,
                exit: output.lines().last().unwrap_or_default().to_owned(),
                text: last_lines(&output, PASSED_TAIL_LINES),
                failed_tests: 0,
            },
            Ok((false, output)) => {
                let failure = check_failure(&output);
                CheckRun {
                    command: command.clone(),
                    passed: false,
                    exit: output.lines().last().unwrap_or_default().to_owned(),
                    text: failure.text,
                    failed_tests: failure.failed,
                }
            }
            Err(e) => CheckRun { command: command.clone(), passed: false, exit: format!("{e:#}"), text: format!("{e:#}"), failed_tests: 0 },
        };
        tracing::info!(
            target: crate::trace::PROGRESS,
            "check {}: {} · {}",
            if run.passed { "passed" } else { "failed" },
            run.command,
            run.exit
        );
        let stop = !run.passed;
        runs.push(run);
        if stop {
            break;
        }
    }
    runs
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
    /// The project's turn cap that ended the run, when one did: the card continues in a fresh
    /// session instead of being judged.
    capped: Option<u32>,
    /// The files the run read, one entry per read, relative to the workspace.
    reads: Vec<String>,
}

/// What a tool call did, for the handoff of a run its cap ended: the tool and the path or the
/// command it took, `write_file hello.txt`; the tool alone when it took neither.
fn call_summary(call: &ra_providers::ToolCall) -> String {
    let arguments = serde_json::from_str::<serde_json::Value>(&call.arguments).unwrap_or_default();
    match arguments["path"].as_str().or_else(|| arguments["command"].as_str()) {
        Some(what) => format!("{} {}", call.name, what.trim()),
        None => call.name.clone(),
    }
}

/// The report a run its cap ended leaves for the next run of the card, which starts fresh: how
/// it ended, what changed, what the worker said and did, and what is left, which is the whole
/// acceptance since no check ran.
fn capped_report(cap: u32, session: &Session, changed: &[String], node: &Node) -> String {
    let mut text = format!(
        "the turn cap of {cap} ended the run after {} turn(s), {} tool call(s); the card continues in a fresh session\n",
        session.turns, session.calls
    );
    if changed.is_empty() {
        text.push_str("changed: nothing\n");
    } else {
        let _ = writeln!(text, "changed: {}", changed.join(", "));
    }
    let acceptance = node.acceptance.as_deref().map(str::trim).filter(|a| !a.is_empty()).unwrap_or("none written");
    let _ = writeln!(text, "left: the check did not run; the acceptance stands: {acceptance}");
    if !session.text.trim().is_empty() {
        let _ = write!(text, "\n{}", session.text.trim());
    }
    text
}

/// The size of the turns for the run line and the handoff: the context a turn carried is the
/// cost of the run, so the average tokens a turn read and wrote leads, as `avg 12k tokens a
/// turn, largest 41k`; the largest only when the provider showed each turn, the average from the
/// sum over the turns when it showed only that.
fn turn_sizes(session: &Session) -> String {
    let average = average_turn(session);
    let largest = session.turn_usage.iter().map(TurnUsage::total).max();
    match largest {
        Some(largest) => format!("avg {} tokens a turn, largest {}", kilo(average), kilo(largest)),
        None => format!("avg {} tokens a turn", kilo(average)),
    }
}

/// The tokens an average turn of the session read and wrote: over the turns the provider showed,
/// else the sum over the turns it counted.
fn average_turn(session: &Session) -> u64 {
    let shown: Vec<u64> = session.turn_usage.iter().map(TurnUsage::total).collect();
    match u64::try_from(shown.len()).unwrap_or(0) {
        0 => (session.usage.prompt_tokens + session.usage.completion_tokens) / session.turns.max(1),
        count => shown.iter().sum::<u64>() / count,
    }
}

/// How many files a run line names of the ones a run read again.
const REREAD_SHOWN: usize = 3;

/// The files a run read more than once, the most re-read first, at most [`REREAD_SHOWN`] of
/// them, as `c.rs ×12, x86.rs ×9, abi.rs ×7`; nothing when every file was read once.
fn reread(reads: &[String]) -> Option<String> {
    let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for path in reads {
        *counts.entry(path.as_str()).or_default() += 1;
    }
    let mut again: Vec<(&str, usize)> = counts.into_iter().filter(|(_, n)| *n > 1).collect();
    if again.is_empty() {
        return None;
    }
    again.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    Some(again.iter().take(REREAD_SHOWN).map(|(path, n)| format!("{path} ×{n}")).collect::<Vec<_>>().join(", "))
}

/// A token count in short: `812`, `4.1k`, `120k`.
fn kilo(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1000..=9999 if n % 1000 < 100 => format!("{}k", n / 1000),
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

/// The verdict of a run against its estimate, for the run line and the handoff: the estimate's
/// tokens a turn against the run's average, `over the estimate of 8k a turn at 400k a turn` or
/// `under the estimate of …`; the total is not the measure, the context a turn carried is.
fn estimate_verdict(estimate: Estimate, average: u64) -> String {
    let side = if estimate.is_over(average) { "over" } else { "under" };
    format!("{side} the estimate of {} a turn at {} a turn", kilo(estimate.a_turn()), kilo(average))
}

/// What to do with the build after a run, from the files it changed, for the run line and the
/// handoff: a change to the window's own code (`apps/rc-gui`, or the `ra-console` code it links)
/// shows only after `rc restart`, which waits for the milestone commit, when a phase's cards are
/// done and committed together, or for the person's ask; a change to `rc` or the other crates is
/// live for the next run after an `rc install` at any time; a run that touched no Rust code says
/// nothing.
fn build_advice(changed: &[String]) -> Option<&'static str> {
    let path = |p: &String| p.replace('\\', "/");
    if changed.iter().map(path).any(|p| p.starts_with("apps/rc-gui/") || p.starts_with("crates/ra-console/")) {
        Some("window: restart on the milestone commit")
    } else if changed.iter().map(path).any(|p| p.starts_with("apps/rc/") || p.starts_with("crates/")) {
        Some("rc: install any time, no restart")
    } else {
        None
    }
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
/// the workspace, the results go back, until the hard cap of `caps` runs out when there is one,
/// or the summed cost crosses `budget` when there is one. The restart cap of `caps` ends the run
/// too, but as a session to continue from: what the model said and the calls it made stand in
/// for its answer, and the card goes on in a fresh session.
async fn tool_loop(
    provider: &dyn ModelProvider,
    worker: &Worker,
    id: i64,
    context: String,
    workspace: Option<&Path>,
    budget: Option<f64>,
    caps: Caps,
) -> anyhow::Result<Session> {
    let workspace = workspace.context("the worker has no workspace to act in")?;
    let toolbox = worker.toolbox(workspace)?;
    let mut messages = vec![Message::system(SYSTEM), Message::user(context)];
    let mut usage = Usage::default();
    let mut turn_usage = Vec::new();
    let mut calls = 0usize;
    let mut turn = 0u32;
    let mut reads: Vec<String> = Vec::new();
    // what the worker said and did, for the handoff when the restart cap ends the run
    let mut said: Vec<String> = Vec::new();
    let mut did: Vec<String> = Vec::new();
    let limit = caps.limit();
    loop {
        turn += 1;
        if let Some(cap) = caps.hard
            && turn > cap
        {
            anyhow::bail!(
                "the worker made {calls} tool call(s) in {cap} turns without finishing; raise the cap with --turns or split the task"
            );
        }
        if let Some(restart) = caps.restart
            && turn > restart
        {
            let mut text = said.join("\n");
            if !did.is_empty() {
                let _ = write!(text, "\ncalls: {}", did.join(" · "));
            }
            let text = text.trim().to_owned();
            return Ok(Session { text, usage, calls, turns: u64::from(restart), turn_usage, cap: limit, capped: Some(restart), reads });
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
                max_turns: limit,
                effort: None,
            })
            .await
            .map_err(map_error)?;
        // every turn is a completion of its own, so each lands in the ledger as it happens
        worker.record(Some(id), &answer.usage, 1).await;
        add_usage(&mut usage, &answer.usage);
        turn_usage.push(TurnUsage::of(&answer.usage));
        if answer.tool_calls.is_empty() {
            return Ok(Session { text: answer.text, usage, calls, turns: u64::from(turn), turn_usage, cap: limit, capped: None, reads });
        }
        // the budget is checked before the tools run: the work it would pay for is not done
        if let Some((budget, spent)) = budget.zip(usage.cost_usd).filter(|(budget, spent)| spent > budget) {
            return Err(OverBudget { budget, spent, turns: turn, calls }.into());
        }
        if !answer.text.trim().is_empty() {
            said.push(answer.text.trim().to_owned());
        }
        did.extend(answer.tool_calls.iter().map(call_summary));
        messages.push(Message::assistant_calls(answer.text, answer.tool_calls.clone()));
        for call in &answer.tool_calls {
            tracing::debug!("#{id} turn {turn}: {}({})", call.name, call.arguments);
            if call.name == "read_file"
                && let Some(path) =
                    serde_json::from_str::<serde_json::Value>(&call.arguments).ok().and_then(|a| a["path"].as_str().map(str::to_owned))
            {
                reads.push(path);
            }
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
        if assessed_mark(node) == Some(fingerprint(node, &nodes).as_str()) {
            receipts.push(format!("#{} {} assessed and unchanged: no call", node.id, node.title));
            unchanged.push(format!("#{}", current));
            continue;
        }
        // a tree pass over a milestone also asks what cards its objective still lacks
        let propose = recursive && node.kind == NodeKind::Milestone;
        let connected = match provider.clone() {
            Some(connected) => connected,
            None => {
                let connected = worker.provider()?;
                provider = Some(connected.clone());
                connected
            }
        };
        match reassess_one(store, worker, connected.as_ref(), current, &nodes, propose).await {
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
/// person's standing direction, the node's own notes and its children with their states, so the
/// revision names only the work the subtree still owes. With `propose`, the model may add the
/// cards the objective lacks and, while gambits stand, name a child no gambit covers as one to
/// park; both wait in the proposals file for the person's word, less the ones declined before.
/// Nothing is parked on the model's say-so.
async fn reassess_one(
    store: &Store,
    worker: &Worker,
    provider: &dyn ModelProvider,
    id: i64,
    nodes: &[Node],
    propose: bool,
) -> anyhow::Result<String> {
    let node = store.read(id).await?;
    let old_objective = node.objective.clone();
    let old_acceptance = node.acceptance.clone();
    let direction = direction_lines(&store.gambits().await?);
    let mut ask = context_of(store, &node).await?.text;
    if !direction.is_empty() {
        ask.push('\n');
        ask.push_str(&ra_context::with_direction("", &direction));
    }
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
    if propose {
        ask.push_str(
            "\nThis node is a milestone. When its objective needs work that no child covers, add one line per missing \
             card after the three, each as:\ncard: <title> | <objective, one line> | <acceptance, one line>\n\
             Add none when the children cover the objective.",
        );
        if !direction.is_empty() {
            ask.push_str(
                "\nWhen a child card serves none of the person's standing direction above, you may propose parking it, one \
                 line each, as:\npark: #<its id> | <why no direction covers it, one line>\nThe person decides; propose \
                 none when every child serves a direction.",
            );
        }
    }
    let answer = provider
        .complete(CompletionRequest {
            model: worker.model.clone(),
            messages: vec![Message::system(ARCHITECT), Message::user(ask)],
            temperature: Some(0.2),
            max_tokens: Some(if propose { 1024 } else { 512 }),
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
    let mut proposals: Vec<Proposal> = Vec::new();
    for line in answer.text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("card:") {
            if let Some(proposal) = Proposal::parse(rest) {
                proposals.push(proposal);
            }
        } else if let Some(rest) = line.strip_prefix("park:") {
            // only a child still open can be parked, and only while a direction stands to miss
            if propose
                && !direction.is_empty()
                && let Some(proposal) = Proposal::parse_park(rest, nodes.iter().filter(|n| n.parent == Some(id)))
            {
                proposals.push(proposal);
            }
        } else if let Some(rest) = line.strip_prefix("overview:") {
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
        store.set_assessed(id, assessed_line(revised, Some(&fingerprint(revised, &after))).as_deref()).await?;
    }
    // what the person declined before, and what a child already is, does not come back as new;
    // a park is keyed by the card it names, so a reworded reason is the same proposal
    if propose {
        let declined: Vec<&str> = declined_of(&node).collect();
        let mut seen: Vec<String> = nodes.iter().filter(|n| n.parent == Some(id)).map(|n| n.title.to_lowercase()).collect();
        proposals.retain(|p| {
            let key = p.park.map_or_else(|| p.title.to_lowercase(), |card| format!("park #{card}"));
            let fresh = !declined.contains(&declined_mark(&key).as_str()) && !seen.contains(&key);
            seen.push(key);
            fresh
        });
        save_proposals(store, id, &proposals)?;
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
    if !proposals.is_empty() {
        for line in proposal_lines(id, &proposals) {
            receipt.push('\n');
            receipt.push_str(&line);
        }
    }
    Ok(receipt)
}

/// A card the architect found missing under a milestone, or one it found serving none of the
/// person's standing direction, waiting for the person's word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    pub title: String,
    pub objective: String,
    pub acceptance: String,
    /// The child this proposal would park, when it is a park rather than a new card; its title
    /// is `title` and the architect's reason `objective`. Accepting blocks the card: it stays on
    /// the roadmap and out of the walk.
    pub park: Option<i64>,
}

impl Proposal {
    /// A `card:` line of the architect's answer: title, objective and acceptance between bars; a
    /// line without a real title is noise.
    fn parse(rest: &str) -> Option<Self> {
        let mut parts = rest.split('|').map(str::trim);
        let title = parts.next().filter(|t| !t.is_empty() && !is_placeholder(t))?;
        let field = |value: Option<&str>| value.filter(|v| !is_placeholder(v)).unwrap_or("").to_owned();
        Some(Self { title: title.to_owned(), objective: field(parts.next()), acceptance: field(parts.next()), park: None })
    }

    /// A `park:` line of the architect's answer: `#<id> | <reason>`, where the id is one of
    /// `children` and the card is still open; anything else is noise.
    fn parse_park<'a>(rest: &str, children: impl IntoIterator<Item = &'a Node>) -> Option<Self> {
        let (id, reason) = rest.split_once('|').map_or((rest.trim(), ""), |(id, reason)| (id.trim(), reason.trim()));
        let id: i64 = id.trim_start_matches('#').parse().ok()?;
        let child = children.into_iter().find(|n| n.id == id)?;
        if matches!(child.state, TaskState::Implemented | TaskState::Accepted | TaskState::Blocked | TaskState::Cancelled) {
            return None;
        }
        let reason = if is_placeholder(reason) { String::new() } else { reason.to_owned() };
        Some(Self { title: child.title.clone(), objective: reason, acceptance: String::new(), park: Some(id) })
    }
}

/// Where the proposals of a milestone wait: one per line, the fields tab-separated, so the window
/// and the console read the same list.
fn proposals_file(store: &Store, id: i64) -> PathBuf {
    workspace_of(store).join("var").join("proposals").join(format!("{id}.txt"))
}

/// The proposals waiting on `id`, in the order the architect gave them.
#[must_use]
pub fn proposals_of(store: &Store, id: i64) -> Vec<Proposal> {
    let Ok(text) = std::fs::read_to_string(proposals_file(store, id)) else { return Vec::new() };
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let title = parts.next()?.trim();
            (!title.is_empty()).then(|| Proposal {
                title: title.to_owned(),
                objective: parts.next().unwrap_or("").to_owned(),
                acceptance: parts.next().unwrap_or("").to_owned(),
                park: parts.next().and_then(|p| p.trim().parse().ok()),
            })
        })
        .collect()
}

/// Writes the proposals of `id`; an empty list removes the file, so nothing stale waits.
fn save_proposals(store: &Store, id: i64, proposals: &[Proposal]) -> anyhow::Result<()> {
    let path = proposals_file(store, id);
    if proposals.is_empty() {
        if path.exists() {
            std::fs::remove_file(&path).with_context(|| format!("cannot remove {}", path.display()))?;
        }
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    let flat = |text: &str| text.replace(['\t', '\n'], " ");
    let text: String = proposals
        .iter()
        .map(|p| {
            let park = p.park.map(|id| id.to_string()).unwrap_or_default();
            format!("{}\t{}\t{}\t{park}\n", flat(&p.title), flat(&p.objective), flat(&p.acceptance))
        })
        .collect();
    std::fs::write(&path, text).with_context(|| format!("cannot write {}", path.display()))
}

/// The list the person reads: a lead naming the commands, then one numbered line per proposal;
/// a park names the card it would set aside and the architect's reason.
#[must_use]
pub fn proposal_lines(id: i64, proposals: &[Proposal]) -> Vec<String> {
    let mut out = vec![format!(
        "? {} card(s) proposed under #{id}; /accept {id} <n> takes one, /decline {id} <n> drops it for good",
        proposals.len()
    )];
    for (n, p) in proposals.iter().enumerate() {
        out.push(match p.park {
            Some(card) => format!(
                "  {}. park #{card} {} — {}",
                n + 1,
                p.title,
                if p.objective.is_empty() { "no standing direction covers it" } else { &p.objective }
            ),
            None => format!("  {}. {} — {}", n + 1, p.title, if p.objective.is_empty() { "no objective" } else { &p.objective }),
        });
    }
    out
}

/// Takes proposal `n` (counted from 1) off the list of `id`, and writes the rest back.
fn take_proposal(store: &Store, id: i64, n: usize) -> anyhow::Result<(Proposal, Vec<Proposal>)> {
    let mut proposals = proposals_of(store, id);
    anyhow::ensure!(!proposals.is_empty(), "no proposals wait on #{id}; a reassess-tree of a milestone makes them");
    anyhow::ensure!((1..=proposals.len()).contains(&n), "the proposals of #{id} are 1 to {}", proposals.len());
    let taken = proposals.remove(n - 1);
    save_proposals(store, id, &proposals)?;
    Ok((taken, proposals))
}

/// Accepts proposal `n` of milestone `id`: a task under it with the proposed text, or the named
/// card blocked with the reason on its notes when the proposal was a park; then the list that
/// still waits.
pub async fn accept_proposal(store: &Store, id: i64, n: usize) -> anyhow::Result<Vec<String>> {
    let (taken, rest) = take_proposal(store, id, n)?;
    fn field(text: &str) -> Option<&str> {
        (!text.is_empty()).then_some(text)
    }
    let first = match taken.park {
        Some(card) => {
            // blocked keeps the card on the roadmap and out of the walk; the reason goes on its
            // notes, so a later reader knows why it stopped
            store.set_state(card, TaskState::Blocked).await?;
            if !taken.objective.is_empty() {
                store.set_notes(card, &format!("parked on the person's word: {}", taken.objective)).await?;
            }
            format!("#{card} {} parked under #{id}: blocked on your word, `/set {card} defined` brings it back", taken.title)
        }
        None => {
            let new = store.insert(Some(id), NodeKind::Task, &taken.title, field(&taken.objective), field(&taken.acceptance), 3).await?;
            format!("#{new} {} added under #{id}", taken.title)
        }
    };
    let mut out = vec![first];
    if !rest.is_empty() {
        out.extend(proposal_lines(id, &rest));
    }
    Ok(out)
}

/// Declines proposal `n` of milestone `id`: its title's hash goes on the `assessed:` line, so no
/// later reassessment proposes it again.
pub async fn decline_proposal(store: &Store, id: i64, n: usize) -> anyhow::Result<Vec<String>> {
    let node = store.read(id).await?;
    let (taken, rest) = take_proposal(store, id, n)?;
    let mark = declined_mark(&taken.park.map_or_else(|| taken.title.clone(), |card| format!("park #{card}")));
    if !declined_of(&node).any(|h| h == mark) {
        let line = format!("{} -{mark}", assessed_line(&node, assessed_mark(&node)).unwrap_or_default());
        store.set_assessed(id, Some(line.trim())).await?;
    }
    let mut out = vec![match taken.park {
        Some(card) => format!("declined parking #{card} {}: it stays, and is not proposed again", taken.title),
        None => format!("declined {}: it does not return", taken.title),
    }];
    if !rest.is_empty() {
        out.extend(proposal_lines(id, &rest));
    }
    Ok(out)
}

/// The mark of a node's `assessed:` line: its first word, the fingerprint or `final`, when it has
/// one; a line that holds only declined proposals has none.
fn assessed_mark(node: &Node) -> Option<&str> {
    node.assessed.as_deref().and_then(|a| a.split_whitespace().next()).filter(|w| !w.starts_with('-'))
}

/// The hashes of the proposals the person declined on the node: the `-` words of its `assessed:`.
fn declined_of(node: &Node) -> impl Iterator<Item = &str> {
    node.assessed.as_deref().into_iter().flat_map(str::split_whitespace).filter_map(|w| w.strip_prefix('-'))
}

/// The `assessed:` line that puts `mark` before the declined proposals the node keeps; `None`
/// when neither is there.
#[must_use]
pub fn assessed_line(node: &Node, mark: Option<&str>) -> Option<String> {
    let words: Vec<String> = mark.map(str::to_owned).into_iter().chain(declined_of(node).map(|h| format!("-{h}"))).collect();
    (!words.is_empty()).then(|| words.join(" "))
}

/// The hash under which a declined title is remembered: of the title lowercased, so a reworded
/// case does not bring it back.
fn declined_mark(title: &str) -> String {
    hash_fields([title.trim().to_lowercase().as_str()])
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

/// The brief of a card: the prompt text, and the files it works in, which the outlines read.
struct Brief {
    text: String,
    /// The card's files, or those it inherits from the nearest ancestor that names any.
    files: Vec<String>,
}

/// The prompt of one task: the node, its ancestors, and its dependencies. One read of the store
/// serves them all, and names the files a card without its own takes from a section above it.
async fn context_of(store: &Store, node: &Node) -> anyhow::Result<Brief> {
    let nodes = store.all().await?;
    let find = |id: i64| nodes.iter().find(|n| n.id == id);
    let mut text = format!("Task #{}: {}\n", node.id, node.title);
    if let Some(objective) = &node.objective {
        text.push_str(&format!("Objective: {objective}\n"));
    }
    if let Some(acceptance) = &node.acceptance {
        text.push_str(&format!("Acceptance criteria: {acceptance}\n"));
    }
    // the files the task names, or its section's: where the change lands; the task context
    // outlines them, and the card view says whose they are
    let (files, from) = ra_roadmap::tree::files_of(&nodes, node);
    if !files.is_empty() {
        match from {
            Some(owner) => {
                let _ = writeln!(text, "Files of the card, where the change lands, from #{} {}:", owner.id, owner.title);
            }
            None => text.push_str("Files of the card, where the change lands:\n"),
        }
        for file in files {
            let _ = writeln!(text, "  {file}");
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
    Ok(Brief { text, files: files.to_vec() })
}

/// Maps a provider error to a refusal that names what to do.
pub(crate) fn map_error(error: ProviderError) -> anyhow::Error {
    match error {
        ProviderError::Auth(_) => anyhow::anyhow!("the provider refused the key: check OPENROUTER_API_KEY, or set one with key <paste>"),
        ProviderError::UnknownModel(model) => anyhow::anyhow!("the provider knows no model {model}"),
        ProviderError::Unavailable(text) => anyhow::anyhow!("the provider is unreachable: {text}"),
        ProviderError::TurnsOut(answer) => anyhow::anyhow!(
            "the run failed: the worker ran out of turns after {} turn(s) without finishing; raise the cap with --turns or split the task",
            answer.turns
        ),
        ProviderError::Other(text) => anyhow::anyhow!("the run failed: {text}"),
    }
}

/// A cost for a log line: the dollars, or a word when the provider reported none.
fn cost_text(cost: Option<f64>) -> String {
    cost.map_or_else(|| "cost unreported".to_owned(), |c| format!("${c:.4}"))
}

/// The model id without its vendor.
pub(crate) fn short(model: &str) -> &str {
    model.rsplit('/').next().unwrap_or(model)
}

#[cfg(test)]
mod tests;
