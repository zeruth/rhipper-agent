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
use ra_roadmap::store::{Card, Store};
use ra_roadmap::tree;
use ra_usage::{Entry, Totals, UsageStore};
use tracing::Instrument as _;

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
/// How many characters of a dependency's notes a brief carries: the review at their head.
const DEPENDENCY_NOTES_ROOM: usize = 2000;

/// The short names of the Claude models and the ids they stand for on the Claude Code route: the
/// ids move with the releases, the names stay, and a person types the name.
pub const ALIASES: [(&str, &str); 4] =
    [("fable", "claude-fable-5-2"), ("opus", "claude-opus-5-5"), ("sonnet", "claude-sonnet-5-5"), ("haiku", ra_providers::route::HAIKU)];

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
    /// What the ladder did to the route of the card this worker is for, `escalated to <model>
    /// after 1 rejection` or `started one rung down (c2)`, which the start line and the handoff
    /// say; `None` when the card runs on its route's model.
    pub step: Option<String>,
    /// Whether a run on a `claude-*` model goes ahead past the plan's threshold, from `--force`;
    /// off, such a run falls to the local route when one is up and is refused otherwise.
    pub force: bool,
    /// Debug mode, on unless `RHIPPER_DEBUG=off`: a run asks the worker to end its report with
    /// what would have made it faster, keeps the answer, and hands the latest to the next runs.
    pub debug: bool,
    /// What answers the commands of a run instead of the shell; the tests script the check and
    /// the lint with it.
    script: Option<ra_tools::Script>,
    /// Whether a dispatch dry-runs the acceptance's commands first; `None` is on for the shell
    /// and off for a scripted worker, whose script counts the commands it is asked.
    preflight: Option<bool>,
    /// The cheap roles when a test set them; a run resolves its own from `rhipper.toml` and the
    /// local server otherwise.
    pub(crate) roles: Option<Roles>,
    /// The directory the worker acts in and the checks run in when it is not the project's: a
    /// worktree of the card. The roadmap, the context files and `var/` stay the project's.
    workspace: Option<PathBuf>,
    /// The branch of that worktree, which the check line and the handoff name.
    branch: Option<String>,
    /// What a fix pass works from, when the worker is one: the instruction with the reviewer's
    /// verdict or the lint's warnings, leading the task instead of the card being taken up
    /// afresh.
    fix: Option<String>,
    /// The install directory a walk watches between cards, moving to a newer `rc` found there;
    /// `None`, the default and the tests', watches none.
    install: Option<PathBuf>,
    /// What starts the walk again on the installed `rc`; the tests hand in one that starts nothing.
    relaunch: fn(&Path, &Path) -> anyhow::Result<u32>,
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
            step: None,
            force: false,
            debug: !std::env::var("RHIPPER_DEBUG")
                .is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "off" | "0" | "false" | "no")),
            script: None,
            preflight: None,
            roles: None,
            workspace: None,
            branch: None,
            fix: None,
            install: None,
            relaunch,
        }
    }

    /// The same worker, its walks moving to a newer `rc` installed in `dir` between cards.
    #[must_use]
    pub fn with_install(mut self, dir: Option<PathBuf>) -> Self {
        self.install = dir;
        self
    }

    /// The same worker, its walks started again on a newer install by `relaunch`.
    #[must_use]
    pub fn with_relaunch(mut self, relaunch: fn(&Path, &Path) -> anyhow::Result<u32>) -> Self {
        self.relaunch = relaunch;
        self
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

    /// The same worker, with the dispatch's dry run of the acceptance forced on or off.
    #[must_use]
    pub fn with_preflight(mut self, on: bool) -> Self {
        self.preflight = Some(on);
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

    /// The worker a run of `node` in `root` goes to once the plan's week is read: this one,
    /// unless it is on a `claude-*` model and the week has passed a mark of the project's
    /// `[plan] ladder` that is for the card; then, without `--force`, the same worker on the
    /// rung's model, or on the local server's when the rung says `local` or `hold`. A `local`
    /// rung with no local route up is an error naming the room left and the reset; a `hold`
    /// one waits for the week to reset, as [`wait_for_plan`] waits on the five-hour window.
    ///
    /// # Errors
    /// When the week is past a `local` mark, the run is not forced and no local route is up.
    pub async fn steered_by_plan(&self, root: &Path, node: &Node) -> anyhow::Result<Self> {
        if self.force || self.route() != Route::ClaudeCode {
            return Ok(self.clone());
        }
        let plan = crate::router::Router::load(root).map(|router| router.plan).unwrap_or_default();
        let readings = plan_readings(root);
        // every reading of a week that has not reset; the ladder decides whether it is past a mark
        let Some((name, window)) = plan_week(&readings, 0.0, unix_now()) else {
            return Ok(self.clone());
        };
        let Some((place, mark)) = plan.rung(node, window.utilization) else {
            return Ok(self.clone());
        };
        let week = format!("week at {:.0}%", window.utilization * 100.0);
        let card = format!("c{}", node.complexity);
        let steer = |model: String| {
            tracing::info!(target: crate::trace::PROGRESS, "  steer: {week}, {card} → {} by ladder {place}", short(&model));
            let mut worker = self.clone();
            worker.model = model;
            // the steer line says why the model moved; the ladder's rung is not this one
            worker.step = None;
            worker
        };
        let to = mark.to.trim();
        if !to.eq_ignore_ascii_case(crate::router::TO_LOCAL) && !to.eq_ignore_ascii_case(crate::router::TO_HOLD) {
            let model = resolve(to);
            return Ok(if model == self.model { self.clone() } else { steer(model) });
        }
        // the tests never ping the local server
        let local = if cfg!(test) { None } else { ra_providers::local::default_model().await };
        if let Some(model) = local {
            return Ok(steer(model));
        }
        let at = format!(
            "the plan's {week} as {name} last saw it, past the {:.0}% mark of ladder {place} for {card}: {}",
            mark.at * 100.0,
            ra_providers::limits::room(&window)
        );
        if to.eq_ignore_ascii_case(crate::router::TO_LOCAL) {
            anyhow::bail!("{at}; no local route is up, run with --force to spend the week");
        }
        let held = format!("hold: {at}; this run waits for the reset, or run with --force");
        // the tests never wait out a week
        anyhow::ensure!(!cfg!(test), "{held}");
        wait_for_week(root, mark.at, &held).await;
        Ok(self.clone())
    }

    /// The worker a run of `node` in `root` goes to: this one unless it is routed; then the one
    /// on the model the card, the rules or the default name, moved up or down the ladder by the
    /// card's size and its rejections, with the rule's effort when `--effort` named none.
    ///
    /// # Errors
    /// When `rhipper.toml` is there but cannot be read.
    pub fn for_card(&self, root: &Path, node: &Node) -> anyhow::Result<Self> {
        if !self.routed {
            return Ok(self.clone());
        }
        let (choice, step) =
            crate::router::Router::load(root)?.route(node, crate::router::IMPLEMENTER, &self.model, rejections(root, node.id));
        let mut worker = self.clone();
        worker.model = resolve(&choice.model);
        worker.effort = self.effort.or(choice.effort);
        worker.step = step.map(|step| step.note);
        Ok(worker)
    }

    /// The same worker on a route that can start: when its model's key is missing the model is
    /// Claude Code's, and the step says so. A worker already connected keeps its provider.
    #[must_use]
    pub fn with_usable_route(&self) -> Self {
        let mut worker = self.clone();
        if self.provider.lock().is_some() {
            return worker;
        }
        let chosen = crate::route::choose(&self.model, crate::route::key_present);
        if let Some(note) = chosen.note() {
            worker.model = chosen.model;
            worker.step = Some(worker.step.map_or(note.clone(), |step| format!("{step} · {note}")));
        }
        worker
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

    /// The ledger every completion is written to, when the worker has one: the planner reads
    /// its costliest cards and the walk reads a day of it back against a planned saving.
    #[must_use]
    pub fn ledger(&self) -> Option<&UsageStore> {
        self.ledger.as_ref()
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

    /// The ledger's cards, the costliest first, each summed over every run on it; `None`
    /// without a ledger, or when it cannot be read.
    pub async fn by_card(&self) -> Option<Vec<ra_usage::CardCost>> {
        let ledger = self.ledger.as_ref()?;
        match ledger.by_card().await {
            Ok(cards) => Some(cards),
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

    /// The turns of the card's latest run of the worker's own, from the ledger; `None` without a
    /// ledger, or when the card has no run in it.
    async fn last_run_turns(&self, node: i64) -> Option<u64> {
        let calls = self.ledger.as_ref()?.calls_of(Some(node)).await.ok()?;
        calls.iter().find(|call| call.role.is_none()).map(|call| call.turns)
    }

    /// Books one completion of `turns` turns against the tally and the ledger. A ledger that
    /// cannot be written is a warning, not a failed run: the answer already came back.
    pub(crate) async fn record(&self, node: Option<i64>, usage: &Usage, turns: u64) {
        self.record_as(&self.model, None, node, usage, turns).await;
    }

    /// [`record`](Self::record) with the read-in of a worker's run: the prompt tokens it spent
    /// before its first edit, which `rc usage` ranks the cards by.
    pub(crate) async fn record_run(&self, node: Option<i64>, usage: &Usage, turns: u64, read_in: u64) {
        self.book(entry_of(&self.model, None, node, usage, turns, read_in)).await;
    }

    /// [`record`](Self::record) under `model` rather than the badge's and tagged with `role`: a
    /// cheap role's call lands under the model that took it, so the saving shows in `rc usage`
    /// beside the worker's calls.
    pub(crate) async fn record_as(&self, model: &str, role: Option<&str>, node: Option<i64>, usage: &Usage, turns: u64) {
        self.book(entry_of(model, role, node, usage, turns, 0)).await;
    }

    /// Marks why the card's latest run ended on its ledger row, which `rc history <id>` prints;
    /// a ledger that cannot be written is a warning, as a booking's is.
    pub(crate) async fn end_run(&self, node: i64, reason: ra_context::EndReason) {
        if let Some(ledger) = &self.ledger
            && let Err(e) = ledger.set_reason(node, reason.as_str()).await
        {
            tracing::warn!("cannot write the usage ledger: {e:#}");
        }
    }

    async fn book(&self, entry: Entry) {
        self.tally.lock().add(&entry);
        if let Some(ledger) = &self.ledger
            && let Err(e) = ledger.record(&entry).await
        {
            tracing::warn!("cannot write the usage ledger: {e:#}");
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
            // the project's own commands, environment and reads, from the `[worker]` table of
            // its rhipper.toml, when it has one
            let root = Path::new(".");
            let worker = match crate::router::Router::load(root) {
                Ok(router) => router.worker,
                Err(e) => {
                    tracing::warn!("the worker table is off: {e:#}");
                    crate::router::WorkerConfig::default()
                }
            };
            let mut harness = harness_of(root, &worker, |key| std::env::var_os(key));
            // the machine's tools on the run's PATH, the ones the probe found off it as well
            if let Some(manifest) = crate::hub::manifest::load() {
                harness = harness.with_tool_dirs(manifest.dirs());
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

/// The Claude Code harness of the project at `root` under its `[worker]` table: its own
/// commands, its environment, and the directories outside a worktree its runs may read. `var`
/// reads the environment, so a test names the toolchains it finds.
pub(crate) fn harness_of(
    root: &Path,
    worker: &crate::router::WorkerConfig,
    var: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> ra_providers::claude_code::ClaudeCode {
    let mut harness = ra_providers::claude_code::ClaudeCode::new().with_suite_check(worker.checks_the_suite());
    if !worker.allow.is_empty() {
        harness = harness.with_allowed(worker.rules());
    }
    if !worker.env.is_empty() {
        harness = harness.with_env(worker.env.clone());
    }
    if let Some(shell) = &worker.shell {
        harness = harness.with_shell(shell);
    }
    harness.with_builds_refused(worker.builds() == 0).with_web(worker.web.unwrap_or(false)).with_read_roots(read_roots(root, worker, var))
}

/// The directories outside its worktree a run of the project at `root` may read, absolute and
/// existing: the table's `read` paths, the directories it carries into a worktree (a link the
/// harness may follow to its real place), and the caches of the toolchains the project builds
/// with. About 225 look-backs of 2026-11-12 were reads refused out there.
pub(crate) fn read_roots(
    root: &Path,
    worker: &crate::router::WorkerConfig,
    var: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut add = |path: PathBuf| {
        if path.is_dir()
            && let Ok(path) = std::path::absolute(&path)
            && !roots.contains(&path)
        {
            roots.push(path);
        }
    };
    for entry in worker.read.iter().map(|e| e.trim()).filter(|e| !e.is_empty()) {
        let path = root.join(entry);
        if !path.is_dir() {
            tracing::warn!("[worker] read names {entry}, which is no directory here");
        }
        add(path);
    }
    for entry in worker.carry.iter().map(|e| e.trim()).filter(|e| !e.is_empty()) {
        add(root.join(entry));
    }
    for cache in toolchain_caches(root, &var) {
        add(cache);
    }
    roots
}

/// The dependency and source caches of the toolchains the project at `root` builds with, as the
/// toolchains place them: cargo's registry and git checkouts under its home, rustup's
/// toolchains (the standard library's source), Gradle's caches under its user home, Maven's
/// repository, and the Java home with its `src.zip`. Only the caches: a home's credentials stay
/// out. The ones that do not exist are left to the caller.
fn toolchain_caches(root: &Path, var: &impl Fn(&str) -> Option<std::ffi::OsString>) -> Vec<PathBuf> {
    let home = || {
        let (first, second) = if cfg!(windows) { ("USERPROFILE", "HOME") } else { ("HOME", "USERPROFILE") };
        var(first).or_else(|| var(second)).filter(|h| !h.is_empty()).map(PathBuf::from)
    };
    let home_of = |key: &str, dir: &str| var(key).filter(|v| !v.is_empty()).map(PathBuf::from).or_else(|| home().map(|h| h.join(dir)));
    let has = |files: &[&str]| files.iter().any(|f| root.join(f).is_file());
    let mut caches = Vec::new();
    if has(&["Cargo.toml"]) {
        if let Some(cargo) = home_of("CARGO_HOME", ".cargo") {
            caches.extend([cargo.join("registry"), cargo.join("git")]);
        }
        if let Some(rustup) = home_of("RUSTUP_HOME", ".rustup") {
            caches.push(rustup.join("toolchains"));
        }
    }
    let gradle = has(&["build.gradle", "build.gradle.kts", "settings.gradle", "settings.gradle.kts"]);
    let maven = has(&["pom.xml"]);
    if gradle && let Some(user_home) = home_of("GRADLE_USER_HOME", ".gradle") {
        caches.push(user_home.join("caches"));
    }
    if maven && let Some(home) = home() {
        caches.push(home.join(".m2").join("repository"));
    }
    if (gradle || maven)
        && let Some(java) = var("JAVA_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).or_else(|| java_on_path(var))
    {
        caches.push(java);
    }
    caches
}

/// The Java home of the `java` on the path, its links followed: the directory above its `bin`,
/// when that has a `lib`, as a JDK has.
fn java_on_path(var: &impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<PathBuf> {
    let program = if cfg!(windows) { "java.exe" } else { "java" };
    let found = std::env::split_paths(&var("PATH")?).map(|dir| dir.join(program)).find(|p| p.is_file())?;
    let real = std::fs::canonicalize(&found).ok()?;
    // canonical on Windows is the verbatim form, which the harness reads as a different path
    let real = match real.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(plain) => PathBuf::from(plain),
        None => real,
    };
    let home = real.parent()?.parent()?.to_path_buf();
    home.join("lib").is_dir().then_some(home)
}

/// Sends one message and returns the answer and the usage as log lines.
///
/// # Errors
/// When the connection cannot be established or the ping fails.
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
            max_context: None,
            effort: None,
            read_only: false,
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
            cost_text(&answer.usage)
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
        Check::new("landing", false, landing_check(root).await),
        Check::new("temp", false, temp_check(root)),
        Check::new("line endings", false, crate::setup::line_endings_check(root)),
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
fn begin_run(root: &Path, id: i64, cap: Option<u32>) {
    // SAFETY: the console runs one card at a time and reads no environment on another thread
    // while a run starts
    unsafe {
        std::env::set_var(CARD_VAR, id.to_string());
        match cap {
            Some(cap) => std::env::set_var(turn_cap_warning::CAP_VAR, cap.to_string()),
            None => std::env::remove_var(turn_cap_warning::CAP_VAR),
        }
    };
    let _ = std::fs::remove_file(root.join(READ_HINTS_FILE));
    let _ = std::fs::remove_file(root.join(turn_cap_warning::TURNS_FILE));
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

/// The reason the pre hook refuses a Read, or a shell's `cat` of a whole file, with: a whole read
/// of a long file, the file's items in it, as the project's catalogue says, the seed's when the
/// project has none. No card or context file is asked for: a worktree has neither, and every
/// walk runs in one, so asking for them let every whole read through. Nothing for any other call.
#[must_use]
pub fn read_refusal(root: &Path, call: &serde_json::Value) -> Option<String> {
    let catalogue = Catalogue::refresh(&root.join(GOTCHAS_FILE)).map_or_else(|_| Catalogue::seed(), |(catalogue, _)| catalogue);
    match ra_providers::claude_code::read_verdict(&catalogue, root, call) {
        ra_gotchas::Verdict::Deny { reason, .. } => Some(reason),
        _ => None,
    }
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

/// The local server answering at its URL with the model the roles take on it, or silent, which
/// turns the roles off rather than sending them to a Claude model.
async fn local_check() -> Result<String, String> {
    let url = ra_providers::openrouter::local_url();
    match ra_providers::local::default_model_at(&url).await {
        Some(model) => Ok(format!("{url} answers with {model}")),
        None => Err(format!("{url} does not answer: the roles are off; set {} to a server", ra_providers::openrouter::LOCAL_URL_VAR)),
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
/// so the console draws its steps in a bubble of their own. A run whose checks pass and whose
/// lint alone fails takes the cheap fix pass of [`review`] with the warning lines as its task
/// ([`lint_fix`]) before the reviewer, so the person does not clear a warning by hand.
///
/// # Errors
/// When the card cannot be read or written, or the provider cannot be reached.
pub async fn run(store: &Store, id: i64, worker: &Worker) -> anyhow::Result<Vec<String>> {
    run_picked(store, id, worker, false).await
}

/// [`run`], with whether a tight window picked the card, which the start line and the pair
/// feed then say.
async fn run_picked(store: &Store, id: i64, worker: &Worker, tight: bool) -> anyhow::Result<Vec<String>> {
    // the node first, so a bad id or a project is named before the key matters
    let node = store.read(id).await?;
    anyhow::ensure!(node.kind != NodeKind::Project, "#{id} is a project; run a task");
    // the model of this card, when the command left the choice to the card and the rules
    let root = workspace_of(store);
    // a card the person blocked or cancelled after the walk picked it is theirs: no continuation
    // or rerun starts over it (asm #81's walk started one on a card blocked to be finished by hand)
    if held_by_person(node.state) {
        let line = not_started_line(id, &node.title, node.state);
        tracing::info!(target: crate::trace::PROGRESS, "{line}");
        crate::pair::event(&root, &line);
        return Ok(vec![line]);
    }
    // a command the tool rejects holds the card before any worker is picked or started
    if let Some(lines) = held_by_preflight(store, &root, &node, worker).await? {
        return Ok(lines);
    }
    // a card whose files and passing check are already in the base needs no worker
    if let Some(lines) = skipped_in_base(store, &root, &node, worker).await? {
        return Ok(lines);
    }
    // a card already reported implemented, its tree and notes as that run left them, goes to the
    // reviewer and not to another worker
    if worker.script.is_none() && reported_implemented(store, id).await? {
        let mut lines = vec![format!("  #{id} was reported implemented and nothing changed since: no worker started")];
        if ra_context::TaskContext::open(&root, id).handoffs.last().is_some_and(|h| h.verdict.is_none()) {
            lines.extend(review_after_run(store, id, worker).await);
        }
        return Ok(lines);
    }
    // then the plan's week: a claude-* pick past the threshold falls to the local route or is
    // refused here, before the start marker
    let worker = &worker.for_card(&root, &node)?.steered_by_plan(&root, &node).await?.with_usable_route();
    let mut pick = worker.step.as_ref().map_or_else(String::new, |step| format!(" · {step}"));
    if tight {
        write!(pick, " · {TIGHT_PICK}").unwrap();
    }
    // the cheap roles of this run, resolved once: the local server is pinged here, not per role,
    // and the start marker says where each role went, so the ledger is read against it
    let roles = worker.roles(&root).await;
    let roles_part = roles.summary();
    // the pid on the start marker, last: a window coming up tells a run that died from one
    // still going
    tracing::info!(
        target: crate::trace::PROGRESS,
        "{}#{} {} · {}{pick} · {roles_part} · pid {}",
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
    let mut outcome = run_task(store, node, worker, &roles).await;
    // an absolute card is one run until it is done: a failed check or a rejection sends it
    // straight back to work on its session, the verdict as the prompt, inside this run, not
    // back to the walk's queue for a restart (the person: needs revision "passed around with
    // an absolute card restarting, they shouldn't do that"). A pass that changed no file in
    // three in a row is a card going round stuck; it ends the run and the walk blocks it
    let mut passes = 0;
    loop {
        // a lint-only failure never reaches the reviewer, so its fix pass starts here: the
        // warnings are the task, the pass lints again, and a clean one goes on to the reviewer
        if let Ok((lines, _, _)) = &mut outcome
            && store.read(id).await.is_ok_and(|n| n.state == TaskState::NeedsRevision)
        {
            lint_fix(store, id, worker, &roles, lines).await;
        }
        // the reviewer judges a run that passed its checks before the card is offered:
        // implemented means review-ready, and a card is review-ready only with a verdict attached
        if let Ok((lines, _, _)) = &mut outcome
            && store.read(id).await.is_ok_and(|n| n.state == TaskState::Implemented)
        {
            lines.extend(review_after_run(store, id, worker).await);
        }
        // a run sent back that changed nothing and says why is idle: the next run starts fresh,
        // and a second in a row parks the card (#185 went round seven times on its resumed
        // session, 26k to 93k tokens a turn, every run refused the same command)
        if let Ok((lines, _, _)) = &mut outcome
            && let Some(line) = settle_idle(store, &root, id).await
        {
            lines.push(line);
        }
        let Ok(node) = store.read(id).await else { break };
        if outcome.is_err() || !node.is_absolute() || node.state != TaskState::NeedsRevision {
            break;
        }
        // a send-back split into cards waits on them
        if !node.depends_on.is_empty() {
            let all = store.all().await.unwrap_or_default();
            if node.depends_on.iter().any(|dep| !all.iter().any(|d| d.id == *dep && d.state == TaskState::Accepted)) {
                break;
            }
        }
        if ra_context::TaskContext::open(&root, id).idle_rounds() >= RERUNS_A_WALK {
            break;
        }
        passes += 1;
        tracing::info!(target: crate::trace::PROGRESS, "  absolute: pass {} goes on, the verdict as its prompt", passes + 1);
        let previous = outcome.as_ref().map(|(lines, _, _)| lines.clone()).unwrap_or_default();
        outcome = run_task(store, node, worker, &roles).await.map(|(lines, usage, gotchas)| {
            let mut all = previous;
            all.extend(lines);
            (all, usage, gotchas)
        });
    }
    match &outcome {
        Ok((lines, usage, 0)) => {
            tracing::info!(target: crate::trace::PROGRESS, "{}✓ {}", crate::logs::RUN_END, end_cost(usage));
            crate::pair::event(&root, &format!("end {}", lines.first().map_or("", String::as_str)));
        }
        Ok((lines, usage, gotchas)) => {
            tracing::info!(target: crate::trace::PROGRESS, "{}✓ {} · {gotchas} gotchas", crate::logs::RUN_END, end_cost(usage));
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
const REVIEW_INSTRUCTION: &str = "You review one card of a roadmap after a worker implemented it. You get the card's objective and acceptance, the worker's handoff, the files the card names and the run changed with their diff stat, and the git diff of those files, the output of the card's checks, which passed, and your own last verdict on the card when there was one. A change may live in any file of that list, not only the one you expect; read the stat before judging something missing, and the check output before asking for a run of it. Judge whether the diff meets the acceptance and whether the handoff tells the truth about it. Answer with one word on the first line: `accepted` or `needs_revision`. After `needs_revision`, list the points one per line as `N. [work|card|environment|nit] path:line: what`, every point citing the file and line it is about: `work` is what the worker can change in the project's files and the card needs; `card` is what only the card's own text or the person's decision settles (its acceptance, its files, a choice between designs); `environment` is what the machine lacks and the worker cannot make (another toolchain, a fixture another tool builds, a CI run, a service); `nit` is a matter of taste the card does not need (a name, a wording, a \"consider\"), which never sends the card back. At most five `work` points: name the five worst. Where your last verdict is given, a point of it that still stands keeps its number and its words, a point the change settled is dropped, and a new point takes the next number. After `accepted`, nothing or one line of why. No other text.";

/// How much of the diff the reviewer reads: a card's files, not a reformat of the tree.
const REVIEW_DIFF: usize = 60_000;

/// The reviewer's verdict on a card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Accepted,
    /// The points the reviewer listed, one per entry, without their bullets.
    NeedsRevision(Vec<String>),
    /// An answer whose first line is neither word: no verdict, and no points.
    Malformed,
}

/// The verdict of a reviewer's answer: `accepted` on the first line is acceptance; a
/// `needs_revision` first line is a revision, with the lines after it as its points; a first
/// line that is neither word is [`Verdict::Malformed`].
#[must_use]
pub fn verdict_of(answer: &str) -> Verdict {
    let mut lines = answer.lines().map(str::trim).filter(|line| !line.is_empty());
    let Some(first) = lines.next() else {
        return Verdict::NeedsRevision(vec![NO_ANSWER_POINT.to_owned()]);
    };
    let word_of = |line: &str| line.trim_matches(|c: char| !c.is_alphanumeric()).to_ascii_lowercase().replace(' ', "_");
    let word = word_of(first);
    if word.starts_with("accepted") {
        return Verdict::Accepted;
    }
    let point = |line: &str| {
        let bare = line.trim_start_matches(['-', '*', '•']).trim();
        match tagged(bare) {
            Some((Some(number), kind, what)) => format!("{number}. [{}] {what}", kind.name()),
            Some((None, kind, what)) => format!("[{}] {what}", kind.name()),
            None => bare.to_owned(),
        }
    };
    let rest: Vec<&str> = lines.collect();
    // A reviewer that opens with a sentence and closes with the word accepts.
    if rest.last().is_some_and(|last| word_of(last).starts_with("accepted")) {
        return Verdict::Accepted;
    }
    if !word.starts_with("needs_revision") {
        // prose is no list of points: the worker would be sent to settle an essay
        return Verdict::Malformed;
    }
    // Every point a tick is an acceptance; a mixed list keeps only the failing ones.
    let points: Vec<String> = rest.iter().filter(|line| !is_tick(line)).map(|line| point(line)).collect();
    if points.is_empty() && !rest.is_empty() {
        return Verdict::Accepted;
    }
    Verdict::NeedsRevision(if points.is_empty() { vec![NO_POINTS_POINT.to_owned()] } else { points })
}

/// The point of an empty answer, which [`verdict_of`] writes, not the reviewer.
const NO_ANSWER_POINT: &str = "the reviewer answered nothing";

/// The point of a `needs_revision` with nothing after it, which [`verdict_of`] writes.
const NO_POINTS_POINT: &str = "the reviewer gave no points";

/// The most work points one send-back carries: past them the reviewer is asked for its worst,
/// and the rest are dropped.
const WORK_POINTS: usize = 5;

/// The prefix of a notes line that keeps a nit of the reviewer's: read, never rerun.
const NIT_POINT: &str = "nit: ";

/// Whether a point cites a place: a word of it is a path, `src/a.rs:12` or `hello.txt`. A point
/// with no place could not be found, and "consider renaming" rode reruns beside real points.
#[must_use]
pub fn cites_a_place(point: &str) -> bool {
    let what = tagged(point).map_or(point, |(_, _, what)| what);
    what.split(|c: char| c.is_whitespace() || matches!(c, '`' | '"' | '\'' | ',' | '(' | ')' | ';')).any(is_path)
}

/// Whether a word is a file's path, with a line after it or not: a stem and a short extension
/// with a letter in it, which `1.5` and `...` are not.
fn is_path(word: &str) -> bool {
    let word = word.trim_end_matches(['.', ':']);
    let file = word.split_once(':').map_or(word, |(file, _)| file);
    let Some((stem, ext)) = file.rsplit_once('.') else {
        return false;
    };
    stem.chars().any(char::is_alphanumeric)
        && (1..=5).contains(&ext.len())
        && ext.chars().all(|c| c.is_ascii_alphanumeric())
        && ext.chars().any(|c| c.is_ascii_alphabetic())
}

/// Whether a point is one [`verdict_of`] wrote for want of the reviewer's, which no place settles.
fn is_stand_in(point: &str) -> bool {
    let point = tagged(point).map_or(point, |(_, _, what)| what);
    point == NO_ANSWER_POINT || point == NO_POINTS_POINT
}

/// Whether a point is the worker's to settle: tagged `work`, or not tagged at all.
fn is_work(point: &str) -> bool {
    matches!(point_kind(point), None | Some(PointKind::Work))
}

/// Whether a send-back is asked once more before it stands: a point of the reviewer's cites no
/// place, or more than [`WORK_POINTS`] are work.
#[must_use]
pub fn wants_another_ask(points: &[String]) -> bool {
    points.iter().any(|point| !is_stand_in(point) && !cites_a_place(point))
        || points.iter().filter(|point| is_work(point)).count() > WORK_POINTS
}

/// The verdict once its points were asked for again: a point that still cites no place is
/// dropped, and so are the work points past the first [`WORK_POINTS`]. Nothing left is an
/// acceptance.
#[must_use]
pub fn placed(verdict: Verdict) -> Verdict {
    let Verdict::NeedsRevision(points) = verdict else {
        return verdict;
    };
    let mut work = 0;
    let kept: Vec<String> = points
        .into_iter()
        .filter(|point| is_stand_in(point) || cites_a_place(point))
        .filter(|point| {
            work += usize::from(is_work(point));
            !is_work(point) || work <= WORK_POINTS
        })
        .collect();
    if kept.is_empty() { Verdict::Accepted } else { Verdict::NeedsRevision(kept) }
}

/// A verdict without its nits, and the nits, which go on the notes: a list of nits alone is an
/// acceptance, since a nit never sends a card back.
#[must_use]
pub fn without_nits(verdict: Verdict) -> (Verdict, Vec<String>) {
    let Verdict::NeedsRevision(points) = verdict else {
        return (verdict, Vec::new());
    };
    let (nits, rest): (Vec<String>, Vec<String>) = points.into_iter().partition(|point| point_kind(point) == Some(PointKind::Nit));
    if rest.is_empty() { (Verdict::Accepted, nits) } else { (Verdict::NeedsRevision(rest), nits) }
}

/// Who can settle a point of the reviewer's: the tag it carries, `N. [kind] file: what`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointKind {
    /// The worker can change it in the project's files: a rerun settles it.
    Work,
    /// Only the card's text or the person's decision settles it.
    Card,
    /// The machine lacks it (a toolchain, a fixture, a CI run): no rerun makes it.
    Environment,
    /// A matter of taste the card does not need: it rides the notes and never sends the card back.
    Nit,
}

impl PointKind {
    /// The tag of the kind, as the reviewer writes it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Work => "work",
            Self::Card => "card",
            Self::Environment => "environment",
            Self::Nit => "nit",
        }
    }
}

/// The kind of a point as [`verdict_of`] leaves it; `None` for a point without a tag, which a
/// rerun takes up as work.
#[must_use]
pub fn point_kind(point: &str) -> Option<PointKind> {
    tagged(point).map(|(_, kind, _)| kind)
}

/// Whether a rejection's points are every one the card's or the environment's: no rerun can
/// meet them, so the card waits for the person rather than the walk (asm #80 ran 109 rounds on
/// fixtures another toolchain builds).
#[must_use]
pub fn for_the_person(points: &[String]) -> bool {
    !points.is_empty() && points.iter().all(|point| leaves_the_card(point))
}

/// Whether a point is the card's or the environment's, which no rerun settles.
fn leaves_the_card(point: &str) -> bool {
    matches!(point_kind(point), Some(PointKind::Card | PointKind::Environment))
}

/// The prefix of a notes line that hands an environment point of a mixed rejection to the person.
const PERSON_POINT: &str = "person: ";

/// The prefix of a notes line that names the follow-up card a card point of a mixed rejection
/// became: `follow-up #N: <point>`.
const FOLLOW_UP_POINT: &str = "follow-up #";

/// The characters of a card point a follow-up card takes for its title.
const FOLLOW_UP_TITLE: usize = 80;

/// A mixed rejection split at its source: the work points (and the untagged ones) stay the
/// rerun's; each environment point goes on the notes as a `person:` line, and each card point
/// becomes a follow-up card under the card's parent, waiting on this one, with the point as its
/// objective. The verdict of the work points and the notes lines of the points that left; a
/// list of one kind is left whole. Without the split the whole list rode every rerun, and the
/// worker spent its turns on points only the person could settle (runelite passed 10 of 72).
async fn split_points(store: &Store, node: &Node, verdict: Verdict) -> (Verdict, Vec<String>) {
    let Verdict::NeedsRevision(points) = verdict else {
        return (verdict, Vec::new());
    };
    if for_the_person(&points) || !points.iter().any(|point| leaves_the_card(point)) {
        return (Verdict::NeedsRevision(points), Vec::new());
    }
    let (mut work, mut left) = (Vec::new(), Vec::new());
    for point in points {
        match tagged(&point) {
            Some((_, PointKind::Environment, _)) => left.push(format!("{PERSON_POINT}{point}")),
            Some((_, PointKind::Card, what)) => {
                let title: String = what.chars().take(FOLLOW_UP_TITLE).collect();
                let card = Card { objective: Some(what), waits_on: &[node.id], ..Card::new() };
                match store.insert_card(node.parent, NodeKind::Task, title.trim(), &card).await {
                    Ok(follow) => left.push(format!("{FOLLOW_UP_POINT}{follow}: {point}")),
                    Err(e) => {
                        // a point no card could take still leaves the rerun: the person reads it
                        tracing::warn!("#{} could not add a follow-up card for {point}: {e:#}", node.id);
                        left.push(format!("{PERSON_POINT}{point}"));
                    }
                }
            }
            _ => work.push(point),
        }
    }
    (Verdict::NeedsRevision(work), left)
}

/// How many substantive points of one send-back make the walk file a card per point.
const SPLIT_AT: usize = 4;

/// The prefix of a notes line that names the card a point of a split send-back became.
const SPLIT_POINT: &str = "split to #";

/// The paths a point names: the words of it that are a file of the project under `root`.
fn files_named(root: &Path, point: &str) -> Vec<String> {
    let mut files: Vec<String> = Vec::new();
    for word in point.split(|c: char| c.is_whitespace() || matches!(c, '`' | '"' | '\'' | ',' | '(' | ')' | ';')) {
        let path = word.trim_end_matches(['.', ':']).replace('\\', "/");
        let path = path.split_once(':').map_or(path.as_str(), |(file, _)| file).to_owned();
        if path.contains('.') && root.join(&path).is_file() && !files.contains(&path) {
            files.push(path);
        }
    }
    files
}

/// A substantive send-back of [`SPLIT_AT`] points or more filed as one card per point: each
/// carries its point as its objective and the files the point names, under the card's parent,
/// and the card waits on them all. The verdict then holds no point for a rerun, and the notes
/// lines name the cards. One rerun of the whole list spent its turns on the points one after
/// the other and met the same review again.
async fn split_by_point(store: &Store, node: &Node, root: &Path, verdict: Verdict) -> (Verdict, Vec<String>) {
    let Verdict::NeedsRevision(points) = &verdict else {
        return (verdict, Vec::new());
    };
    if points.len() < SPLIT_AT
        || for_the_person(points)
        || crate::commands::review::classify(points) != crate::commands::review::Weight::Substantive
    {
        return (verdict, Vec::new());
    }
    let mut made = Vec::new();
    for point in points {
        let title: String = point.chars().take(FOLLOW_UP_TITLE).collect();
        let files = files_named(root, point);
        let card = Card { objective: Some(point), files: &files, ..Card::new() };
        let child_result = match store.insert_card(node.parent, NodeKind::Task, title.trim(), &card).await {
            Ok(child) => store.depend_on(node.id, child).await.map(|()| child),
            Err(e) => Err(e),
        };
        match child_result {
            Ok(child) => made.push(format!("{SPLIT_POINT}{child}: {point}")),
            Err(e) => {
                // a point no card could take stays with the card: it reruns on that one
                tracing::warn!("#{} could not file a card for {point}: {e:#}", node.id);
                return (verdict_with_unfiled(&verdict, &made), made);
            }
        }
    }
    (Verdict::NeedsRevision(Vec::new()), made)
}

/// The verdict of a split that stopped partway: the points past the cards made stay.
fn verdict_with_unfiled(verdict: &Verdict, made: &[String]) -> Verdict {
    match verdict {
        Verdict::NeedsRevision(points) => Verdict::NeedsRevision(points.iter().skip(made.len()).cloned().collect()),
        other => other.clone(),
    }
}

/// Whether a line of a card's notes is a point that left the card ([`split_points`]).
fn is_left_point(line: &str) -> bool {
    let point = line.strip_prefix(PERSON_POINT).or_else(|| {
        let (number, point) = line.strip_prefix(FOLLOW_UP_POINT)?.split_once(": ")?;
        number.parse::<i64>().is_ok().then_some(point)
    });
    point.is_some_and(leaves_the_card)
}

/// The points that left the card, each once, in the order of its notes.
fn left_points(notes: &str) -> Vec<&str> {
    let mut left: Vec<&str> = Vec::new();
    for line in notes.lines().filter(|line| is_left_point(line)) {
        if !left.contains(&line) {
            left.push(line);
        }
    }
    left
}

/// `report` with the points that left the card in `notes` ahead of it, so a rerun's notes keep
/// them for the next reviewer, who is told not to raise them again.
fn with_left_points(notes: Option<&str>, report: &str) -> String {
    let left: Vec<&str> = left_points(notes.unwrap_or_default()).into_iter().filter(|line| !report.contains(line)).collect();
    if left.is_empty() { report.to_owned() } else { format!("{}\n\n{report}", left.join("\n")) }
}

/// A point's number, kind and text after the tag: `3. [work] src/a.rs: x`, `3) [Work] …` and
/// `[card] …` read; a point without a known tag is `None`.
fn tagged(point: &str) -> Option<(Option<u32>, PointKind, &str)> {
    let digits = point.find(|c: char| !c.is_ascii_digit()).unwrap_or(point.len());
    let (head, tail) = point.split_at(digits);
    let number = head.parse().ok();
    let rest = if number.is_some() { tail.trim_start_matches(['.', ')', ':']).trim_start() } else { point };
    let (tag, what) = rest.strip_prefix('[')?.split_once(']')?;
    let kind = match tag.trim().to_ascii_lowercase().as_str() {
        "work" => PointKind::Work,
        "card" => PointKind::Card,
        "environment" | "env" => PointKind::Environment,
        "nit" => PointKind::Nit,
        _ => return None,
    };
    Some((number, kind, what.trim()))
}

/// A review point that passes: a check mark first, or `ok` / `passes` last.
fn is_tick(line: &str) -> bool {
    let line = point_text(line);
    let end = line.trim_end_matches(|c: char| !c.is_alphanumeric()).to_ascii_lowercase();
    line.starts_with(['✓', '✔', '☑', '✅']) || end.ends_with(" ok") || end.ends_with("passes") || end == "ok"
}

fn point_text(line: &str) -> &str {
    line.trim_start_matches(['-', '*', '•', ' '])
}

/// What a fix pass is told, ahead of the verdict and the card.
const FIX_INSTRUCTION: &str = "A reviewer sent this card back for the points below, every one of them a lint warning, a format diff or a doc comment. Fix exactly those points in the files they name, run the card's checks, and report in a few lines what you changed; change nothing else, and do not take the card up afresh.";

/// What the fix pass of a lint-only failure is told, ahead of the warnings and the card.
const LINT_FIX_INSTRUCTION: &str = "The lint of this card's run left the warnings below in the lines it changed; the checks passed. Fix exactly those lines in the files the warnings name, run the card's checks, and report in a few lines what you changed; change nothing else, and do not take the card up afresh.";

/// What the handoff and the end line say of a lint-only pass's session.
const LINT_PASS_FRESH: &str = "lint pass: fresh";

/// Whether `worker` is the fix pass of a lint-only failure ([`lint_fix`]).
fn is_lint_pass(worker: &Worker) -> bool {
    worker.fix.as_deref().is_some_and(|fix| fix.starts_with(LINT_FIX_INSTRUCTION))
}

/// The lines a lint pass quotes on either side of a warning's line.
const LINT_AROUND: usize = 3;

/// The warnings a lint pass quotes the lines of; the rest it names alone.
const LINT_QUOTED: usize = 20;

/// The prompt of a lint-only pass: `fix`, the instruction and the warnings, then the lines
/// around each warning's place in `acting`, numbered, so the pass reads those lines and not the
/// card's history (asm #237 resumed 199k tokens a turn for five warnings).
fn lint_prompt(acting: &Path, fix: &str) -> String {
    let mut prompt = fix.to_owned();
    let mut quoted: Vec<(String, usize)> = Vec::new();
    for (file, line) in fix.lines().filter_map(warning_at) {
        if quoted.len() >= LINT_QUOTED || quoted.iter().any(|(f, l)| *f == file && *l == line) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(acting.join(&file)) else { continue };
        let from = line.saturating_sub(LINT_AROUND).max(1);
        let lines =
            text.lines().enumerate().skip(from - 1).take(line + LINT_AROUND + 1 - from).fold(String::new(), |mut out, (at, text)| {
                let _ = writeln!(out, "{:>6}  {text}", at + 1);
                out
            });
        if lines.is_empty() {
            continue;
        }
        if quoted.is_empty() {
            prompt.push_str("\n\nThe lines around each warning, as the tree has them now:\n");
        }
        let _ = write!(prompt, "\n{file}:{line}\n{lines}");
        quoted.push((file, line));
    }
    prompt
}

/// The file and line a line of a warning points at: rustc's ` --> src/a.rs:3:9` or the short
/// format's `src/a.rs:3:9: warning: …`; `None` for any other line.
fn warning_at(line: &str) -> Option<(String, usize)> {
    let line = line.trim_start();
    let at = match line.strip_prefix("--> ") {
        Some(at) => at.trim(),
        None => line.split_once(": warning").map(|(at, _)| at)?,
    };
    let (rest, last) = at.rsplit_once(':')?;
    let last: usize = last.trim().parse().ok()?;
    // a column after the line, or the line alone
    let (file, row) = rest.rsplit_once(':').and_then(|(file, row)| row.parse().ok().map(|row| (file, row))).unwrap_or((rest, last));
    (!file.is_empty() && row > 0).then(|| (file.replace('\\', "/"), row))
}

/// The cheapest rung of `ladder` on the route of `model`: a lint pass needs no stronger model,
/// and stays on the route the card was given. `None` when no rung is on that route.
fn cheapest_on_route(ladder: &crate::router::Ladder, model: &str) -> Option<String> {
    let route = Route::of(&resolve(model));
    ladder.models.iter().map(|rung| resolve(rung)).find(|rung| Route::of(rung) == route)
}

/// The outcome a review leaves on the handoff of a trivially rejected run; a second one on the
/// same card halts the fix passes.
const TRIVIAL_TAG: &str = "needs revision (trivial)";

/// The model of a fix pass: the `fix` role of `rhipper.toml`, else the local server's model when
/// it answers a ping; `None` when the role is off, when no server answers, and in the tests,
/// which never ping. The pass never falls to a Claude model the table did not name: the ledger
/// of 2026-11-12 showed haiku at 9.5M input tokens from passes that fell to it by default.
async fn fix_model(root: &Path) -> Option<String> {
    let named = crate::router::Router::load(root).ok().and_then(|router| router.role.fix);
    match named.as_deref() {
        Some(named) => ra_providers::route::role(Some(named), None),
        None if cfg!(test) => None,
        None => ra_providers::route::role(None, ra_providers::local::default_model().await.as_deref()),
    }
}

/// The fix pass of a run whose checks passed and whose lint alone failed: the warning lines of
/// its handoff are the task, on the cheapest rung of the card's route ([`cheapest_on_route`],
/// else the `fix` role), in a fresh session ([`lint_prompt`]), and the pass runs the checks and the lint
/// again, so a clean lint leaves the card implemented for the reviewer and a second red one
/// leaves it needs revision as the first did. The pass's lines join `lines`: after the run's
/// when the lint stayed red, and before them, with `lint fixed on <model>` on the run line,
/// when it came clean, so the end line is the pass's.
async fn lint_fix(store: &Store, id: i64, worker: &Worker, roles: &Roles, lines: &mut Vec<String>) {
    let root = workspace_of(store);
    let task = ra_context::TaskContext::open(&root, id);
    let warnings = task.last_warnings();
    // a failed check is the person's to read; only the lint's warnings take the pass
    if warnings.is_empty() || task.handoffs.last().is_some_and(|h| h.check.starts_with("check failed")) {
        return;
    }
    // the cheapest rung of the card's route first; a `fix = ""` keeps the pass off
    let router = crate::router::Router::load(&root).ok();
    let off = router.as_ref().and_then(|router| router.role.fix.as_deref()).is_some_and(|fix| fix.trim().is_empty());
    let cheapest = router.as_ref().filter(|_| !off).and_then(|router| cheapest_on_route(&router.ladder, &worker.model));
    let model = match cheapest {
        Some(model) => Some(model),
        None => fix_model(&root).await,
    };
    let Some(model) = model else {
        lines.push("  the fix role is off: the lint warnings wait for the person".to_owned());
        return;
    };
    let fix = format!("{LINT_FIX_INSTRUCTION}\n{}", warnings.join("\n\n"));
    let fixer = Worker { model: resolve(&model), fix: Some(fix), ..worker.clone() };
    let model = short(&fixer.model);
    tracing::info!(target: crate::trace::PROGRESS, "#{id} · lint fix pass on {model}");
    let outcome = match store.read(id).await {
        Ok(node) => run_task(store, node, &fixer, roles).await,
        Err(e) => Err(e),
    };
    match outcome {
        Ok((mut passed, _, _)) if store.read(id).await.is_ok_and(|n| n.state == TaskState::Implemented) => {
            if let Some(first) = passed.first_mut() {
                let _ = write!(first, " · lint fixed on {model}");
            }
            let before = std::mem::replace(lines, passed);
            lines.push(format!("  the run before the lint fix pass on {model}:"));
            lines.extend(before);
        }
        Ok((passed, _, _)) => {
            lines.push(format!("  lint fix pass on {model}"));
            lines.extend(passed);
        }
        Err(e) => {
            lines.push(format!("  the lint fix pass on {model} failed: {}; the card waits for the person", first_line(&format!("{e:#}"))));
        }
    }
}

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
    // under the card's span, so its lines carry the card: untagged, the window took a review
    // from the command line for a run of its own and kept a "worker" card open for it (runelite
    // and asm, 2026-11-12); a tagged line of a card with no live run is dropped there
    let span = tracing::info_span!(target: crate::trace::PROGRESS, crate::trace::CARD_SPAN, id);
    async {
        let rerun = rerun_stale_check(store, id, worker).await;
        let lines = review_to(store, id, worker, TaskState::Accepted).await?;
        Ok::<_, anyhow::Error>(rerun.into_iter().chain(lines).collect())
    }
    .instrument(span)
    .await
}

/// What the stamp on a check output's last line starts with: the tree the check ran on.
const CHECKED_ON: &str = "checked on tree ";

/// The tree of what a worktree holds now, committed or not: the checkpoint the walk commits
/// after a run changes the commit and not this, a commit after the check or a merge changes it.
pub async fn content_tip(path: &Path) -> Option<String> {
    let made = git(path, &["stash", "create"]).await.ok()?;
    let made = made.trim();
    let spec = format!("{}^{{tree}}", if made.is_empty() { "HEAD" } else { made });
    let tree = git(path, &["rev-parse", &spec]).await.ok()?;
    Some(tree.trim().to_owned()).filter(|t| !t.is_empty())
}

/// The check output with the tree it ran on at its end, where the clip to the tail keeps it.
#[must_use]
pub fn stamped(output: &str, tip: Option<&str>) -> String {
    match tip {
        Some(tip) => format!("{}\n{CHECKED_ON}{tip}", output.trim_end()),
        None => output.to_owned(),
    }
}

/// Whether a stored check output was made on another tree than `tip`: one that holds output and
/// names no tree, or another, is stale; no output at all is a card that never ran its checks.
#[must_use]
pub fn check_is_stale(output: &str, tip: &str) -> bool {
    if output.trim().is_empty() {
        return false;
    }
    let stamp = output.trim_end().lines().last().and_then(|line| line.strip_prefix(CHECKED_ON));
    stamp.is_none_or(|stamp| stamp.trim() != tip)
}

/// The card's checks run again in its worktree when its branch holds other content than the
/// last check ran on, before the reviewer reads: asm #143 was judged on the output of a run
/// whose test was committed after it. The line of the rerun; `None` when the stored output
/// is current, there is no worktree or the card has no checks.
async fn rerun_stale_check(store: &Store, id: i64, worker: &Worker) -> Option<String> {
    let root = workspace_of(store);
    let path = worker.workspace.clone().unwrap_or_else(|| root.join(WORKTREES_DIR).join(id.to_string()));
    if !path.join(".git").exists() || same_tree(&root, &path) {
        return None;
    }
    let mut task = ra_context::TaskContext::open(&root, id);
    let tip = content_tip(&path).await?;
    if !check_is_stale(&task.check_output, &tip) {
        return None;
    }
    let node = store.read(id).await.ok()?;
    let Checks { commands, timeout, .. } = checks_of(&root, &node);
    if commands.is_empty() {
        return None;
    }
    tracing::info!(target: crate::trace::PROGRESS, "#{id} · the branch moved since its last check: checks run again");
    let own_target = ra_providers::claude_code::shared_target(&path);
    let toolbox = worker.toolbox(&path).map(|toolbox| {
        toolbox.with_timeout(timeout).with_env(own_target.iter().map(|t| ("CARGO_TARGET_DIR".to_owned(), t.display().to_string())))
    });
    let runs = {
        let _slot = hold_build(&root, &path).await;
        let target = own_target.unwrap_or_else(|| crate::hub::cargo_target(&root));
        let cargo = commands.iter().find(|c| c.trim_start().starts_with("cargo")).cloned();
        let waited = match &cargo {
            Some(_) => guard_target(&root, &path, &target, timeout).await,
            None => Ok(None),
        };
        match waited {
            Ok(_target) => run_checks(&toolbox, &commands, &[]).await,
            Err(text) => vec![CheckRun {
                command: cargo.unwrap_or_default(),
                passed: false,
                exit: "not run".to_owned(),
                text,
                failed_tests: 0,
                failures: Vec::new(),
                tests: Vec::new(),
                first_error: String::new(),
                setup: false,
            }],
        }
    };
    let output: Vec<String> = runs.iter().map(|run| format!("$ {} · {}\n{}", run.command, run.exit, run.text.trim_end())).collect();
    task.set_check_output(&stamped(&output.join("\n\n"), Some(&tip)));
    save_context(store, &root, &task).await;
    let passed = runs.iter().all(|run| run.passed);
    Some(format!("  #{id}: the branch moved since its last check; checks run again: {}", if passed { "passed" } else { "failed" }))
}

/// The reviewer on a card a run just left implemented, before it is offered for acceptance:
/// [`review`] with an accepting verdict leaving the card implemented, the verdict on its
/// handoff, so acceptance is the person's glance at it; a rejection sets needs revision as
/// ever. A reviewer that does not answer leaves the card in review, without a verdict and so
/// not offered: `review <id>` judges it once the reviewer answers. A run in a card's worktree
/// takes the project's branch in first and passes its checks on the merged tree
/// ([`catch_up_for_review`]); a conflict or a failure there sends the card back unreviewed. The
/// lines of the catch-up and the verdict.
async fn review_after_run(store: &Store, id: i64, worker: &Worker) -> Vec<String> {
    let root = workspace_of(store);
    let caught = match catch_up_for_review(store, id, worker).await {
        Ok(caught) => caught,
        Err(lines) => return lines,
    };
    match review_to(store, id, worker, TaskState::Implemented).await {
        Ok(lines) => {
            if let Some(first) = lines.first() {
                crate::pair::event(&root, &format!("review {first}"));
            }
            caught.into_iter().chain(lines).collect()
        }
        Err(error) => {
            let why = first_line(&format!("{error:#}"));
            if let Err(e) = store.set_state_by(id, TaskState::Review, Source::Run).await {
                tracing::warn!("#{id} could not be held in review: {e:#}");
            }
            crate::pair::event(&root, &format!("review #{id} no verdict: {why}"));
            caught
                .into_iter()
                .chain([format!("  no verdict on #{id}: {why} · the card waits in review; `review {id}` judges it")])
                .collect()
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
    let Some(model) = fix_model(&root).await else {
        lines.push("  the fix role is off: the card waits for the person".to_owned());
        return Ok(lines);
    };
    let fixer = Worker { model: resolve(&model), fix: Some(format!("{FIX_INSTRUCTION}\n{verdict}")), ..worker.clone() };
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

/// The characters of an off-form answer the second ask quotes back.
const MALFORMED_CLIP: usize = 1500;

/// What the notes of a card say when its verdict came from the second ask.
const REASKED: &str = "asked again: the first answer was off the form";

/// The turns a review gets: enough to read the card's diff and the files it names.
pub(crate) const REVIEW_TURNS: u32 = 4;

/// The turns of the one retry of a review that ran out of [`REVIEW_TURNS`].
pub(crate) const REVIEW_TURNS_LARGER: u32 = 12;

/// One verdict of the reviewer on a card, as [`review`] describes it, without the fix pass:
/// the lines, and the head of the notes (the verdict with its points) when the rejection was
/// trivial. `accepted` is where an accepting verdict leaves the card ([`review_to`]).
async fn judge(store: &Store, id: i64, worker: &Worker, accepted: TaskState) -> anyhow::Result<(Vec<String>, Option<String>)> {
    let node = store.read(id).await?;
    anyhow::ensure!(node.kind != NodeKind::Project, "#{id} is a project; review a task");
    let root = workspace_of(store);
    // the reviewer is a role of its own: the card's assignee is its implementer, not its judge,
    // and a model judging its own work sent #185 back seven times on the same points
    let router = crate::router::Router::load(&root)?;
    let (mut model, mut by) = router.reviewer(&node, &worker.model, rejections(&root, id));
    // a card held for a reviewer that wrote off the form twice is judged one rung above it
    if let Some(failed) = held_without_verdict(&root, id) {
        model = rung_above(&router.ladder, &failed);
        by = format!("one rung above {}, whose review was off the form", short(&failed));
    }
    let provider = worker.provider_for(&model)?;
    // the diff is read where the run acted: a card run in its worktree left nothing in the
    // project's tree, and the reviewer judged an empty diff (#173, the first parallel walk)
    let acting = worker.workspace.clone().unwrap_or_else(|| {
        let worktree = root.join(WORKTREES_DIR).join(id.to_string());
        if worktree.join(".git").exists() { worktree } else { root.clone() }
    });
    let node = with_branch_files(store, &root, &acting, node).await;
    let brief = review_brief(&root, &acting, &node).await;
    let before = node.state;
    store.set_state_by(id, TaskState::Review, Source::Run).await?;
    let dropped = review_notes(&root, &node).1;
    let dropped = if dropped > 0 { format!(" · {dropped} earlier point(s) dropped, the card's text changed") } else { String::new() };
    tracing::info!(target: crate::trace::PROGRESS, "#{id} {} · review by {} ({by}){dropped}", node.title, short(&model));
    let ask = |turns: u32| CompletionRequest {
        model: model.clone(),
        messages: vec![Message::system(REVIEW_INSTRUCTION), Message::user(&brief)],
        temperature: Some(0.0),
        max_tokens: Some(1500),
        tools: Vec::new(),
        workspace: None,
        budget_usd: None,
        resume: None,
        max_turns: Some(turns),
        max_context: None,
        effort: Some(Effort::default().to_string()),
        read_only: false,
    };
    let mut answer = provider.complete(ask(REVIEW_TURNS)).await;
    // a review out of turns (#224) is asked once more with room to read the files the diff names
    if let Err(ProviderError::TurnsOut(spent)) = answer {
        worker.record_as(&model, Some("reviewer"), Some(id), &spent.usage, spent.turns).await;
        answer = provider.complete(ask(REVIEW_TURNS_LARGER)).await;
    }
    let answer = match answer {
        Ok(answer) => answer,
        Err(error) => {
            // a reviewer that did not answer is no verdict: the card goes back where it was
            store.set_state_by(id, before, Source::Run).await?;
            return Err(map_error(error));
        }
    };
    worker.record_as(&model, Some("reviewer"), Some(id), &answer.usage, answer.turns).await;
    let mut answer = answer;
    let mut verdict = verdict_of(&answer.text);
    let reasked = verdict == Verdict::Malformed;
    if reasked {
        // an answer off the form is asked again once, with the form alone
        let clip: String = answer.text.chars().take(MALFORMED_CLIP).collect();
        let again = CompletionRequest {
            messages: vec![
                Message::system(REVIEW_INSTRUCTION),
                Message::user(format!(
                    "Your answer to the review of this card was not in the form:\n{clip}\n\nGive its verdict again, in the form alone: `accepted` or `needs_revision` on the first line, the points after `needs_revision`, one per line."
                )),
            ],
            ..ask(1)
        };
        match provider.complete(again).await {
            Ok(second) => {
                worker.record_as(&model, Some("reviewer"), Some(id), &second.usage, second.turns).await;
                answer = second;
                verdict = verdict_of(&answer.text);
            }
            Err(error) => {
                store.set_state_by(id, before, Source::Run).await?;
                return Err(map_error(error));
            }
        }
        if verdict == Verdict::Malformed {
            // no verdict: the card stays as it was for the next reviewer; after a run, where
            // implemented is the accepting state the walk lands, it waits in review instead, so
            // no card lands without a verdict
            let held = if accepted == TaskState::Implemented { TaskState::Review } else { before };
            // the mark on the handoff names the reviewer, so the walk's next pass asks the
            // review again of the model one rung up ([`review_held`])
            let mut task = ra_context::TaskContext::open(&root, id);
            if let Some(last) = task.handoffs.last_mut() {
                last.verdict = Some(format!("{NO_VERDICT} {model}"));
                if let Err(e) = task.save(&root) {
                    tracing::warn!("the no-verdict mark of #{id} was not saved on its handoff: {e:#}");
                }
            }
            store.set_state_by(id, held, Source::Run).await?;
            let next = if held == TaskState::Review { "the walk's next pass asks it again one rung up" } else { "for the next review" };
            let line = format!(
                "· #{id} {} no verdict: the review by {} was off the form twice; the card stays {held:?}, {next}",
                node.title,
                short(&model)
            );
            return Ok((vec![line], None));
        }
    }
    // a point with no place, or a list past five work points, is asked for once more; what still
    // has no place then is dropped. An ask that fails leaves the verdict as it came
    if let Verdict::NeedsRevision(points) = &verdict
        && wants_another_ask(points)
    {
        let listed: Vec<String> = points.iter().map(|point| format!("- {point}")).collect();
        let again = CompletionRequest {
            messages: vec![
                Message::system(REVIEW_INSTRUCTION),
                Message::user(format!(
                    "{brief}\n\n## Your verdict on this card\n{}\n\nSome of its points cite no `path:line`, or more than {WORK_POINTS} are work. Give the verdict again in the form: `accepted` or `needs_revision` on the first line, then every point as `N. [work|card|environment|nit] path:line: what`, at most {WORK_POINTS} work points, your worst. A point that cites no place is dropped.",
                    listed.join("\n")
                )),
            ],
            ..ask(1)
        };
        match provider.complete(again).await {
            Ok(second) => {
                worker.record_as(&model, Some("reviewer"), Some(id), &second.usage, second.turns).await;
                let asked = verdict_of(&second.text);
                // an answer off the form keeps the first list, with its points that still have no place
                verdict = placed(if asked == Verdict::Malformed { verdict } else { asked });
            }
            Err(error) => tracing::warn!("#{id} the ask for the points' places failed, the verdict stands as given: {error}"),
        }
    }
    let (verdict, nits) = without_nits(verdict);
    let nits: Vec<String> = nits.iter().map(|nit| format!("{NIT_POINT}{nit}")).collect();
    let (verdict, mut left) = split_points(store, &node, verdict).await;
    let (verdict, filed) = split_by_point(store, &node, &root, verdict).await;
    let split = !filed.is_empty() && matches!(&verdict, Verdict::NeedsRevision(rest) if rest.is_empty());
    left.extend(filed);
    // the points' fingerprint rides the handoff's verdict, so a run that changed nothing and drew
    // the same points again is held on that round, not run a third time; the words the review
    // judged ride beside them, so a rewrite of the card after it, as in a split, shows it stale
    let print = match &verdict {
        Verdict::NeedsRevision(points) => {
            format!("{}{}{}{}", ra_context::POINTS_MARK, points_fingerprint(points), ra_context::BRIEF_MARK, card_print(&node))
        }
        Verdict::Accepted | Verdict::Malformed => String::new(),
    };
    let Judged { mut state, mut head, mut said, trivial } = judged(&verdict, id, &model, &root, accepted);
    if split {
        let cards = left.iter().filter(|line| line.starts_with(SPLIT_POINT)).count();
        said = format!("needs revision (substantive) · split into {cards} card(s)");
    }
    // the notes say the verdict came from the second ask; the first answer's usage is booked above
    if reasked {
        head = match head.split_once('\n') {
            Some((first, points)) => format!("{first} · {REASKED}\n{points}"),
            None => format!("{head} · {REASKED}"),
        };
    }
    // the points that left the card follow the rerun's, past the block the rerun reads
    if !left.is_empty() {
        head = format!("{head}\n{}", left.join("\n"));
        for line in &left {
            crate::pair::event(&root, &format!("#{id} {line}"));
        }
    }
    // the verdict rides the run's handoff: a card is offered for acceptance only with one
    // attached, and the weight of a rejection retags the outcome, a trivial one for a cheap
    // pass, a substantive one for the person. A card reviewed before any run has no handoff,
    // and the notes carry the verdict alone
    let mut task = ra_context::TaskContext::open(&root, id);
    if let Some(last) = task.handoffs.last_mut() {
        last.verdict = Some(format!("{said} by {model}{print}"));
        if matches!(state, TaskState::NeedsRevision | TaskState::Blocked)
            && let Some((tag, _)) = said.split_once(" · ")
        {
            tag.clone_into(&mut last.outcome);
        }
        if let Err(e) = task.save(&root) {
            tracing::warn!("the verdict of #{id} was not saved on its handoff: {e:#}");
        }
    }
    // a card the reviewer keeps sending back is held for the person, not queued again
    if state == TaskState::NeedsRevision
        && let Verdict::NeedsRevision(points) = &verdict
        && let Some(why) = task.review_hold()
    {
        let list: Vec<String> = points.iter().map(|point| format!("- {point}")).collect();
        let reason = format!("blocked by the review loop: {why}");
        head = format!("{reason}\nrecurring points:\n{}", list.join("\n"));
        let text = format!("#{id} {reason}: {}; a person sets it defined again", points.join("; "));
        tracing::warn!("{text}");
        crate::pair::event(&root, &text);
        state = TaskState::Blocked;
    }
    // the verdict leads the notes when it is the next reader's task: a rejection's points for
    // the next run, or an acceptance the person asked for. An acceptance after a run leaves the
    // notes to the worker's report; the handoff carries it
    let report = node.notes.as_deref().map(str::trim).filter(|report| !report.is_empty());
    let mut notes = if state == TaskState::Implemented {
        report.map(str::to_owned)
    } else {
        Some(report.map_or_else(|| head.clone(), |report| format!("{head}\n\n{report}")))
    };
    // the nits are kept for the person to read, past the report, where no rerun takes them up
    if !nits.is_empty() {
        let kept = nits.join("\n");
        notes = Some(notes.map_or(kept.clone(), |notes| format!("{notes}\n\n{kept}")));
    }
    if let Some(notes) = notes
        && (state != TaskState::Implemented || !nits.is_empty())
    {
        store.set_notes(id, &notes).await?;
    }
    store.set_state_by(id, state, Source::Run).await?;
    let mut lines = vec![format!(
        "{} #{} {} {said} by {} ({by}) · in {} out {} · {}",
        if verdict == Verdict::Accepted { "✓" } else { "✗" },
        id,
        node.title,
        short(&model),
        answer.usage.prompt_tokens,
        answer.usage.completion_tokens,
        cost_text(&answer.usage)
    )];
    if let Verdict::NeedsRevision(points) = &verdict {
        lines.extend(points.iter().map(|point| format!("  - {point}")));
    }
    lines.extend(left.iter().chain(&nits).map(|line| format!("  {line}")));
    Ok((lines, trivial))
}

/// Where a verdict leaves a card: its state, the head of its notes, the verdict's words for
/// the handoff and the end line, and the head again when the rejection was trivial.
struct Judged {
    state: TaskState,
    head: String,
    said: String,
    trivial: Option<String>,
}

/// The [`Judged`] of `verdict` by `model`; `accepted` is where an acceptance leaves the card.
/// A rejection whose points only the person can settle goes on the feed as well.
fn judged(verdict: &Verdict, id: i64, model: &str, root: &Path, accepted: TaskState) -> Judged {
    // a malformed verdict never gets here: `judge` leaves the card as it was
    let Verdict::NeedsRevision(points) = verdict else {
        return Judged { state: accepted, head: format!("review by {model}: accepted"), said: "accepted".to_owned(), trivial: None };
    };
    let list: Vec<String> = points.iter().map(|point| format!("- {point}")).collect();
    // points only the person can settle block the card: the reason leads the notes, which the
    // person's list reads, and the walk does not take a blocked card up again
    if for_the_person(points) {
        let reason = format!("blocked by the reviewer: {} point(s) for the person, the card's or the environment's", points.len());
        let head = format!("{reason}\nreview by {model}: needs revision (person)\n{}", list.join("\n"));
        let text = format!("#{id} {reason}: {}; a person sets it defined again", points.join("; "));
        tracing::warn!("{text}");
        crate::pair::event(root, &text);
        return Judged {
            state: TaskState::Blocked,
            head,
            said: format!("blocked · {} point(s) for the person", points.len()),
            trivial: None,
        };
    }
    let weight = crate::commands::review::classify(points);
    let head = format!("review by {model}: needs revision ({})\n{}", weight.name(), list.join("\n"));
    let trivial = (weight == crate::commands::review::Weight::Trivial).then(|| head.clone());
    let said = format!("needs revision ({}) · {} point(s)", weight.name(), points.len());
    Judged { state: TaskState::NeedsRevision, head, said, trivial }
}

/// The reviewer's view of the last round: the output of the card's checks and its own last
/// verdict, whose points it keeps by number. Without them a reviewer asked for a run the check
/// had made, and its points rotated round after round (asm #53: 5, 5, 6, 4).
fn last_round(task: &ra_context::TaskContext, notes: &str, dropped: usize) -> String {
    let mut text = String::new();
    if !task.check_output.trim().is_empty() {
        let _ = writeln!(text, "\n{}\n{}", ra_context::CHECK_OUTPUT_HEADING, task.check_output.trim_end());
    }
    // points that judged other words of the card are dropped, and the reviewer is told so
    if dropped > 0 {
        let _ = writeln!(
            text,
            "\n## Earlier review points dropped\n{dropped} point(s) of earlier reviews judged the card before its objective or acceptance changed; they are not in this brief. Judge the card's current text and its diff alone."
        );
    }
    let fresh = task.review_shown().filter(|review| !review.starts_with(ra_context::STALE_REVIEW));
    let last_verdict = ra_context::review_of(notes).or(fresh);
    if let Some(verdict) = last_verdict {
        let _ =
            writeln!(text, "\n## Your last verdict on this card\nKeep the number of every point that still stands.\n{}", verdict.trim());
    }
    let left = left_points(notes);
    if !left.is_empty() {
        let _ = writeln!(
            text,
            "\n## Points that left the card\nThese went to the person or to a follow-up card; do not raise them again.\n{}",
            left.join("\n")
        );
    }
    text
}

/// What the reviewer reads: the card, the last handoff when there is one, and the diff of the
/// card's whole work against its base, clipped: the card's files and every handoff's changed
/// ones ([`card_changed`]), and in a worktree everything the branch changed since its merge
/// base and every untracked file ([`crate::review::scope`]); in the project's tree the tree's
/// other dirty files are listed apart and kept out of the diff.
/// `project` holds the card's context (the handoffs live under the project's `var`); `root`
/// is where the run acted and the diff is read: the project, or the card's worktree.
pub async fn review_brief(project: &Path, root: &Path, node: &Node) -> String {
    let mut text = format!("# #{} {}\n", node.id, node.title);
    if let Some(objective) = &node.objective {
        let _ = writeln!(text, "\nObjective: {objective}");
    }
    let _ = writeln!(text, "\nAcceptance: {}", node.acceptance.as_deref().unwrap_or("none written; the workspace tests are the check"));
    let mut task = ra_context::TaskContext::open(project, node.id);
    task.card_print = Some(card_print(node));
    let (notes, dropped) = review_notes(project, node);
    match task.handoffs.last() {
        Some(last) => {
            let _ = writeln!(text, "\n## Handoff of the last run\n{} on {} · {}\n{}", last.outcome, last.model, last.check, last.report);
        }
        None => text.push_str("\n## Handoff of the last run\nnone: no run of this card left one\n"),
    }
    text.push_str(&last_round(&task, &notes, dropped));
    // the diff's base: HEAD in the project's own tree; in a card's worktree the merge base
    // with the project's branch, so the checkpoints the walk commits on the card's branch
    // after each run are in the reviewer's view and not behind it
    let base = diff_base(project, root).await;
    let shared = same_tree(project, root);
    // the card's whole work, not the last run's: every run's changed files, and on the card's
    // branch everything it changed since its base and every untracked file (runelite #44 was
    // sent back four times for files an earlier checkpoint had landed; #182, #183); in a shared
    // tree the dirty files no handoff of the card names are someone else's
    let handed: Vec<String> = task.handoffs.iter().flat_map(|handoff| handoff.changed.iter().cloned()).collect();
    let stray: Vec<String> =
        if shared { dirty_paths(&git_status(root).await).into_iter().filter(|path| !handed.contains(path)).collect() } else { Vec::new() };
    let mut named: Vec<String> = node.files.iter().filter(|f| !stray.contains(f)).cloned().collect();
    named.extend(handed);
    let files = crate::review::scope(root, &base, !shared, &named).await;
    let changed: Vec<String> = files.iter().filter(|file| !node.files.contains(file)).cloned().collect();
    // the list and the stat come before the diff: the reviewer of #159 looked for the change in
    // the one file it expected and judged the card unfinished for code that lived in another
    text.push_str(&crate::review::files_section(&node.files, &changed, &stray));
    if !files.is_empty() {
        text.push_str(&crate::review::diff_stat_section(root, &base, &files).await);
    }
    // the notes hold the worker's report whole where the handoff holds its summary
    if !notes.trim().is_empty() {
        let notes: String = notes.chars().take(DIGEST_NOTES).collect();
        let _ = writeln!(text, "\n## Notes of the card\n{notes}");
    }
    let _ = writeln!(text, "\n## git diff {base} of the card's files");
    if files.is_empty() {
        text.push_str("the card names no files and no run changed any\n");
        return text;
    }
    let mut command = tokio::process::Command::new("git");
    command.args(["diff", &base, "--"]).args(&files).current_dir(root);
    let diff = match command.output().await {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).into_owned(),
        Ok(out) => format!("git diff failed: {}", String::from_utf8_lossy(&out.stderr).trim()),
        Err(e) => format!("git diff did not run: {e}"),
    };
    if diff.trim().is_empty() {
        let _ = writeln!(text, "no change against {base} in {}", files.join(", "));
    } else if diff.chars().count() > REVIEW_DIFF {
        let cut: String = diff.chars().take(REVIEW_DIFF).collect();
        let _ = writeln!(text, "{cut}\n… clipped at {REVIEW_DIFF} characters");
    } else {
        text.push_str(&diff);
    }
    // a file the run created is not in a diff against HEAD until it is staged: the reviewer
    // rejected a card whole for work it could not see (#143). The new files among the card's
    // come in full, clipped the same way
    let mut untracked = tokio::process::Command::new("git");
    untracked.args(["ls-files", "--others", "--exclude-standard", "--"]).args(&files).current_dir(root);
    if let Ok(out) = untracked.output().await
        && out.status.success()
    {
        let listed = String::from_utf8_lossy(&out.stdout);
        let mut budget = REVIEW_DIFF;
        for path in listed.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let Ok(content) = std::fs::read_to_string(root.join(path)) else { continue };
            let _ = writeln!(text, "\n## new file {path} (not yet tracked)");
            let shown: String = content.chars().take(budget).collect();
            budget = budget.saturating_sub(shown.chars().count());
            text.push_str(&shown);
            if budget == 0 {
                text.push_str("\n… the new files are clipped here\n");
                break;
            }
            text.push('\n');
        }
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
            cost_text(&self.usage)
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
    let model = match crate::router::Router::load(&root)?.pick(&node, crate::router::PLANNER, rejections(&root, node.id)) {
        Some((_, rule)) => resolve(&rule.model),
        None => worker.model.clone(),
    };
    let provider = worker.provider_for(&model)?;
    let brief = plan_brief(store, &node, worker).await?;
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
            max_context: None,
            effort: Some(Effort::default().to_string()),
            read_only: false,
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

/// The evidence block the planner reads beside the project context: the look-backs, the shared
/// machine notes, the ledger's costliest cards of the last day when the worker has a ledger,
/// and the other projects' digests, as [`ra_context::plan::block`] clips them; empty without
/// any.
pub async fn plan_evidence(root: &Path, worker: &Worker) -> String {
    let mut sources = ra_context::plan::Sources::under(root, &crate::hub::dir());
    if let Some(ledger) = worker.ledger()
        && let Ok(cards) = ledger.costliest_cards(ra_usage::LAST_DAY, crate::plan::evidence::LEDGER_CARDS).await
    {
        sources.ledger = cards
            .iter()
            .map(|card| ra_context::plan::LedgerCard {
                node: card.node,
                cost_usd: card.totals.cost_usd,
                turns: card.totals.turns,
                tokens_per_turn: card.tokens_per_turn(),
            })
            .collect();
    }
    ra_context::plan::block(&sources)
}

/// What the planner reads: the project context with the standing direction, the evidence of
/// the runs so far, the node with its objective and acceptance, and the cards already under it
/// with their states, so none comes back twice.
async fn plan_brief(store: &Store, node: &Node, worker: &Worker) -> anyhow::Result<String> {
    let mut text = String::new();
    let context = project_direction(store).await?;
    if !context.trim().is_empty() {
        let _ = writeln!(text, "# Project context\n{}", context.trim_end());
    }
    let evidence = plan_evidence(&workspace_of(store), worker).await;
    if !evidence.is_empty() {
        let _ = writeln!(text, "\n{}", evidence.trim_end());
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
        landed_today: crate::hub::landed_today(root).await,
        waiting: u32::try_from(crate::hub::waiting(root, &nodes).len()).unwrap_or(u32::MAX),
    });
}

/// A unix time as the clock of the day, `hh:mmZ`: UTC, which every project's run reads alike.
fn clock(when: u64) -> String {
    let day = when % 86_400;
    format!("{:02}:{:02}Z", day / 3600, day / 60 % 60)
}

/// Takes the hub's lock on landing in `root`'s checkout, waiting while another process lands
/// there; the wait is said once. `None` when the lock cannot be opened: the landing goes
/// unguarded, as before the lock.
async fn guard_landing(root: &Path) -> Option<crate::hub::Guard> {
    let name = format!("land-{}", crate::hub::project_name(&crate::hub::project_root_of(root)).to_ascii_lowercase());
    let mut said = false;
    loop {
        match crate::hub::try_guard(&name) {
            Ok(Some(guard)) => return Some(guard),
            Ok(None) => {}
            Err(e) => {
                tracing::warn!("the landing runs without its lock: {e:#}");
                return None;
            }
        }
        if !said {
            said = true;
            tracing::info!(target: crate::trace::PROGRESS, "  wait: another landing is merging into the project");
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

/// Takes the hub's lock on the cargo `target` for the check of a run acting in `acting`,
/// waiting on a check that builds in the same target; the wait is said once, naming the
/// holder, on the progress channel and the paired feed, and gives up at `timeout`, the check's
/// own bound (four checks queued behind one another's target for forty minutes on 2026-11-12).
/// `Ok(None)` when the lock cannot be opened: the check goes unguarded.
///
/// # Errors
/// The check's failure when the wait ran out: how long it waited, on what, held by whom.
async fn guard_target(
    root: &Path,
    acting: &Path,
    target: &Path,
    timeout: std::time::Duration,
) -> Result<Option<crate::hub::Guard>, String> {
    let name = crate::hub::target_lock_name(target);
    let who = crate::hub::builder_name(acting);
    let mut said = false;
    let started = std::time::Instant::now();
    loop {
        match crate::hub::try_guard_as(&name, &who) {
            Ok(Some(guard)) => return Ok(Some(guard)),
            Ok(None) => {}
            Err(e) => {
                tracing::warn!("the check runs without the lock on {}: {e:#}", target.display());
                return Ok(None);
            }
        }
        let holder = crate::hub::target_holder(target).unwrap_or_else(|| "a build the hub does not name".to_owned());
        if started.elapsed() >= timeout {
            return Err(format!("the check waited {} s on {} held by {holder}", timeout.as_secs(), target.display()));
        }
        if !said {
            said = true;
            let text = format!("wait: the check waits on the cargo target {}, held by {holder}", target.display());
            tracing::info!(target: crate::trace::PROGRESS, "  {text}");
            crate::pair::event(root, &text);
        }
        let left = timeout.saturating_sub(started.elapsed());
        tokio::time::sleep(std::time::Duration::from_secs(2).min(left)).await;
    }
}

/// The longest a build waits for a slot before it runs without one: a slot's holder may be a
/// gradle that hung, or an `rc hold` orphaned when its tool call was killed, and two of those
/// would otherwise stop every build of the project for good. The two slots run in parallel;
/// the wait only keeps a third build from joining them.
const HOLD_WAIT: std::time::Duration = std::time::Duration::from_mins(15);

/// Takes one of the project's build slots for the check, waiting on the builds of the workers
/// and the other checks; the wait is said once, on the progress channel and the paired feed of
/// the project at `root`. The slot's note names `building`, the tree the check builds, a card's
/// worktree or the project, with its target. `None` when the locks cannot be opened: the check
/// goes unguarded.
async fn hold_build(root: &Path, building: &Path) -> Option<crate::hub::Guard> {
    let mut said = false;
    let started = std::time::Instant::now();
    loop {
        match crate::hub::try_build_slot(building) {
            Ok(Some(guard)) => return Some(guard),
            Ok(None) => {}
            Err(e) => {
                tracing::warn!("the check runs without a build slot: {e:#}");
                return None;
            }
        }
        if started.elapsed() >= HOLD_WAIT {
            tracing::warn!("no build slot came free in {} minutes; the check runs without one", HOLD_WAIT.as_secs() / 60);
            return None;
        }
        if !said {
            said = true;
            let text = format!(
                "wait: the check waits for a worker slot, {} of the machine's {} are running",
                crate::hub::builds_running(),
                crate::hub::build_slot_count()
            );
            tracing::info!(target: crate::trace::PROGRESS, "  {text}");
            crate::pair::event(root, &text);
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

/// The builds one session of a worker may run: the code comes first, and the project's check
/// compiles after the report, so a build of the worker's own is for a compile it cannot read
/// its way to. The person's rule of 2026-11-12: "they should be focusing on code first…
/// report changes without wasting all the local resources building a bunch".
pub const BUILDS_A_SESSION: u32 = 2;

/// Whether a worker's session may run a build now, for the hook: `Ok` spends one of the
/// session's [`BUILDS_A_SESSION`] and `Err` is the refusal's reason. A tree with nothing
/// changed yet is refused outright (a build before the first edit measures nothing the card
/// did not already say), and so is a session past its budget. The count lives in the tree's
/// `var/builds/<session>`; a tree that cannot keep it allows the build. A `command` that names
/// test filters ([`names_tests`]) is a diagnosis of a failure the worker has read: it is allowed
/// on an unchanged tree and is not counted.
///
/// # Errors
/// The reason the build is refused, for the worker.
pub fn build_allowance(tree: &Path, session: &str, command: &str) -> Result<(), String> {
    // the project's table, read above a worktree, which has no copy of it
    let allowed = crate::router::Router::load(&crate::hub::project_root_of(tree)).map_or(BUILDS_A_SESSION, |r| r.worker.builds());
    if allowed == 0 {
        return Err(String::from(
            "the builds are the orchestrator's in this project, not the workers': report the change as it stands, naming what you verified by reading; the project's check compiles it on a build slot after your report, and a failure comes back to this session with the compiler's output",
        ));
    }
    if names_tests(command) {
        return Ok(());
    }
    // the tree's `var/` is the orchestrator's, not the card's change
    let changed = std::process::Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=all", "--", ".", ":(exclude)var"])
        .current_dir(tree)
        .output()
        .map(|out| !String::from_utf8_lossy(&out.stdout).trim().is_empty());
    if matches!(changed, Ok(false)) && !review_asked_test_run(tree, command) {
        return Err(String::from(
            "nothing is changed yet: write the card's change first, then build once if a compile is in doubt; the project's check compiles after the report and a failure comes back as a rerun with its output",
        ));
    }
    let file = builds_file(tree, session);
    let spent = builds_spent(tree, session);
    if spent >= allowed {
        return Err(format!(
            "this session has built {spent} time(s), its budget: report the change as it stands and name what is unverified; the project's check compiles after the report, and a failure comes back as a rerun with its output"
        ));
    }
    if file.parent().is_some_and(|dir| std::fs::create_dir_all(dir).is_ok()) {
        let _ = std::fs::write(&file, (spent + 1).to_string());
    }
    Ok(())
}

/// Whether `command` is a `cargo test` that the card's last verdict, a needs-revision, asked for
/// in its points: a rerun may run it before its first edit, counted as any build is.
fn review_asked_test_run(tree: &Path, command: &str) -> bool {
    if !command.contains("cargo test") {
        return false;
    }
    let root = crate::hub::project_root_of(tree);
    let Some(id) = card_of_run(&root) else {
        return false;
    };
    let Some(review) = ra_context::TaskContext::open(&root, id).review else {
        return false;
    };
    // the stored block is `review by <model> …` over its `- ` points, not the reviewer's answer
    if review.lines().any(|l| l == ra_context::STALE_REVIEW) || !review.lines().next().is_some_and(|l| l.contains("needs revision")) {
        return false;
    }
    review.lines().filter(|l| l.starts_with("- ")).any(|point| {
        let point = point.to_lowercase();
        ["cargo test", "run the test", "run test", "test run", "tests run"].iter().any(|ask| point.contains(ask))
    })
}

/// Whether a shell command is a build the session's budget counts: gradle, cargo or the
/// contract runner under `rc hold`, by the hook's rewrite or the worker's own hand. The other
/// heavy work the slots hold (a script, a search, a WSL toolchain) is the worker's to run.
#[must_use]
pub fn is_build(command: &str) -> bool {
    let inner = command.trim_start().strip_prefix("rc hold").map(|rest| {
        let rest = rest.trim_start();
        rest.split_once("-- ").map_or(rest, |(_, command)| command)
    });
    inner.is_some_and(|command| {
        let first = command.trim_start().trim_start_matches("& ").trim_start_matches(".\\").trim_start_matches("./");
        first.starts_with("gw.ps1")
            || first.starts_with("gradlew")
            || (first.starts_with("cargo ") && !first.starts_with("cargo fmt"))
            || command.contains("run-awt-contracts.py")
    })
}

/// The hook's verdict on `command` once the session's build budget has its say: a build the
/// catalogue lets start spends one of the session's builds, or is refused when they are spent;
/// a command the catalogue refuses, or one that is no build, spends nothing. Only a build that
/// starts counts (every call of the hook spent one before, refused or not, and a session that
/// had run nothing found its budget gone).
#[must_use]
pub fn budget_build(tree: &Path, session: &str, command: &str, verdict: ra_gotchas::Verdict) -> ra_gotchas::Verdict {
    let built = match &verdict {
        ra_gotchas::Verdict::Rewrite { command: rewritten, .. } => is_build(rewritten).then_some(rewritten.as_str()),
        ra_gotchas::Verdict::Pass => is_build(command).then_some(command),
        ra_gotchas::Verdict::Deny { .. } => None,
    };
    let Some(built) = built else {
        return verdict;
    };
    match build_allowance(tree, session, built) {
        Ok(()) => verdict,
        Err(reason) => ra_gotchas::Verdict::Deny { id: String::from("build-budget"), reason },
    }
}

/// The builds a session has spent in `tree`.
#[must_use]
pub fn builds_spent(tree: &Path, session: &str) -> u32 {
    std::fs::read_to_string(builds_file(tree, session)).ok().and_then(|t| t.trim().parse().ok()).unwrap_or(0)
}

/// The file a session's build count lives in: `var/builds/<session>` of the tree.
fn builds_file(tree: &Path, session: &str) -> PathBuf {
    tree.join("var").join("builds").join(session.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect::<String>())
}

/// Probes the machine at a walk's start when the hub's manifest is not today's, off the
/// runtime's threads: the first walk of the day of any project pays for it.
async fn probe_the_machine(root: &Path) {
    let root = root.to_path_buf();
    let probed = tokio::task::spawn_blocking(move || crate::hub::manifest::daily(&root)).await;
    if let Ok(Some(manifest)) = probed {
        tracing::debug!(
            "the machine's manifest of {}: {} tool(s)",
            manifest.day,
            manifest.tools.iter().filter(|t| t.path.is_some()).count()
        );
    }
}

/// The daily runs of a project's slow suites, one line each: the time, the commit of main, the
/// command and the tests it passed, tab-separated, the names space-separated.
pub const SLOW_LOG: &str = "var/slow.log";

/// When the slow suites last started, in unix seconds: a run that named no test still counts,
/// so a broken suite costs one run a day, not one a walk.
const SLOW_STAMP: &str = "var/slow.stamp";

/// How long after their last start the slow suites are due again.
const SLOW_EVERY: u64 = 86_400;

/// The flight's id for the slow suites: no card is numbered below 1.
const SLOW_SEAT: i64 = -1;

/// The failure lines a slow suite's card quotes.
const SLOW_FAILURE_LINES: usize = 8;

/// Whether the slow suites of the project at `root` are due at `now`: it names some under
/// `[worker] slow` and they have not started in the last day.
#[must_use]
pub fn slow_due(root: &Path, now: u64) -> bool {
    let named = crate::router::Router::load(root).is_ok_and(|r| r.worker.slow.iter().any(|c| !c.trim().is_empty()));
    let last = std::fs::read_to_string(root.join(SLOW_STAMP)).ok().and_then(|t| t.trim().parse::<u64>().ok());
    named && last.is_none_or(|last| now.saturating_sub(last) >= SLOW_EVERY)
}

/// Runs the project's slow suites on main when they are due at `now` ([`slow_due`]), each on a
/// worker slot under the check timeout, records what each passed in [`SLOW_LOG`], and files a
/// priority-1 card under the Look-back milestone for every test that passed in the last run of
/// its command and fails now. A landing runs the workspace tests only, so these broke on main
/// unseen (asm's oracle suite, 2026-11-15). One line per suite and per card filed.
pub async fn slow_suites(store: &Store, worker: &Worker, now: u64) -> Vec<String> {
    let root = workspace_of(store);
    if !slow_due(&root, now) {
        return Vec::new();
    }
    let router = crate::router::Router::load(&root).unwrap_or_default();
    let commands: Vec<String> = router.worker.slow.iter().map(|c| c.trim().to_owned()).filter(|c| !c.is_empty()).collect();
    let timeout = router.worker.check_timeout();
    let _ = std::fs::create_dir_all(root.join("var"));
    let _ = std::fs::write(root.join(SLOW_STAMP), now.to_string());
    let commit = git(&root, &["rev-parse", "HEAD"]).await.unwrap_or_default();
    let mut lines = Vec::new();
    let toolbox = match worker.toolbox(&root) {
        Ok(toolbox) => toolbox.with_timeout(timeout),
        Err(e) => return vec![format!("slow suites not run: {e:#}")],
    };
    for command in &commands {
        let output = {
            let _slot = hold_build(&root, &root).await;
            let _target = if command.starts_with("cargo") {
                match guard_target(&root, &root, &crate::hub::cargo_target(&root), timeout).await {
                    Ok(guard) => guard,
                    Err(why) => {
                        lines.push(format!("slow suite `{command}` not run: {why}"));
                        continue;
                    }
                }
            } else {
                None
            };
            match toolbox.check_acceptance(command).await {
                Ok((_, output)) => output,
                Err(e) => {
                    lines.push(format!("slow suite `{command}` could not run: {e:#}"));
                    continue;
                }
            }
        };
        let gradle = !command.starts_with("cargo");
        let passed = passed_test_names(&output);
        let failed = failed_test_names(&output, gradle);
        // a suite that named no test did not run: its last record stays the one to compare with
        if passed.is_empty() && failed.is_empty() {
            lines.push(format!("slow suite `{command}` named no test: {}", first_error(&output)));
            continue;
        }
        let log = std::fs::read_to_string(root.join(SLOW_LOG)).unwrap_or_default();
        let before = last_slow_run(&log, command);
        let mut text = log;
        let _ = writeln!(text, "{now}\t{commit}\t{command}\t{}", passed.join(" "));
        let _ = std::fs::write(root.join(SLOW_LOG), text);
        lines.push(format!("slow suite `{command}`: {} passed, {} failed", passed.len(), failed.len()));
        let Some((green, was)) = before else { continue };
        let range = if commit.is_empty() { format!("{green}..HEAD") } else { format!("{green}..{commit}") };
        let landings: Vec<String> =
            git(&root, &["log", "--format=%h %s", &range]).await.map(|out| out.lines().map(str::to_owned).collect()).unwrap_or_default();
        for test in failed.iter().filter(|t| was.contains(t)) {
            let failure = slow_failure_lines(&output, test);
            match file_slow_failure(store, command, test, &failure, &landings, (&green, &commit)).await {
                Ok(id) => {
                    let line = format!("#{id} filed: `{test}` of the slow suite `{command}` fails on main");
                    crate::pair::event(&root, &line);
                    lines.push(line);
                }
                Err(e) => lines.push(format!("the card for `{test}` could not be filed: {e:#}")),
            }
        }
    }
    lines
}

/// The commit and the passed tests of the last run of `command` in a [`SLOW_LOG`] text.
fn last_slow_run(log: &str, command: &str) -> Option<(String, Vec<String>)> {
    log.lines().rev().find_map(|line| {
        let mut fields = line.splitn(4, '\t');
        let (_, commit, run, passed) = (fields.next()?, fields.next()?, fields.next()?, fields.next().unwrap_or_default());
        (run == command).then(|| (commit.to_owned(), passed.split_whitespace().map(str::to_owned).collect()))
    })
}

/// The names of the tests a cargo or gradle output marks passed: `test name ... ok`, or gradle's
/// `Class > method PASSED` as `Class.method`. A `-q` cargo run names none.
#[must_use]
pub fn passed_test_names(output: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for line in output.lines().map(str::trim) {
        let name = if let Some(rest) = line.strip_prefix("test ") {
            rest.strip_suffix(" ... ok").map(str::to_owned)
        } else {
            line.strip_suffix(" PASSED")
                .and_then(|head| head.split_once(" > "))
                .map(|(class, method)| format!("{class}.{}", method.trim_end_matches("()")))
        };
        if let Some(name) = name
            && !name.is_empty()
            && !names.contains(&name)
        {
            names.push(name);
        }
    }
    names
}

/// The first lines of `test`'s failure in `output`, else the output's first error.
fn slow_failure_lines(output: &str, test: &str) -> String {
    let failures = ra_context::failures_of(output);
    let found = failures.iter().find(|f| f.lines().next().is_some_and(|first| first.contains(test)));
    match found {
        Some(failure) => failure.lines().take(SLOW_FAILURE_LINES).collect::<Vec<_>>().join("\n"),
        None => first_error(output),
    }
}

/// Files the card of a slow suite's test that broke between the commits `green` and `red`:
/// priority 1, under the Look-back milestone, its check the suite filtered to the test.
async fn file_slow_failure(
    store: &Store,
    command: &str,
    test: &str,
    failure: &str,
    landings: &[String],
    (green, red): (&str, &str),
) -> anyhow::Result<i64> {
    let parent = crate::plan::home::ensure_milestone(store).await?;
    let short = |commit: &str| commit.chars().take(10).collect::<String>();
    let landed = if landings.is_empty() { "none git could name".to_owned() } else { landings.join("\n") };
    let objective = format!(
        "The slow suite `{command}`, run on main once a day, passed {test} at {} and fails it at {}.\n\nThe first failure lines:\n{failure}\n\nThe landings since the last green run:\n{landed}",
        short(green),
        short(red)
    );
    let check = if command.starts_with("cargo") { format!("{command} {test}") } else { command.to_owned() };
    let acceptance = format!("`{check}` passes");
    let card = Card { objective: Some(&objective), acceptance: Some(&acceptance), ..Card::new() };
    let id = store.insert_card(Some(parent), NodeKind::Task, &format!("{test} fails on main in the slow suite"), &card).await?;
    store.set_priority(id, Some(1)).await?;
    Ok(id)
}

/// When the walk last read the project's CI, in unix seconds: a read that found no gh counts,
/// so a machine without one pays for the look once an hour, not once a tick.
const CI_STAMP: &str = "var/ci.stamp";

/// How long after a read the CI on main is read again.
const CI_EVERY: u64 = 3600;

/// The workflows a red run filed a card for, one line each: the workflow and the card,
/// tab-separated. A green run of the workflow takes its line out, so a red streak files once.
const CI_FILED: &str = "var/runs/ci-red";

/// Where gh lives when it is not on PATH.
const GH_DEFAULT: &str = r"C:\Program Files\GitHub CLI\gh.exe";

/// The fields of `gh run list` a CI read takes.
const CI_FIELDS: &str = "databaseId,workflowName,headSha,status,conclusion,url,displayTitle";

/// How a CI read calls gh: the arguments in, what gh printed when it exited zero out.
pub type Gh = Arc<dyn Fn(&[&str]) -> Option<String> + Send + Sync>;

/// The latest completed run of a workflow on main, as `gh run list` says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CiRun {
    pub workflow: String,
    /// `success`, `failure`, `timed_out`, `cancelled`…
    pub conclusion: String,
    pub commit: String,
    pub id: u64,
    pub url: String,
    /// The commit's subject, as the run shows it.
    pub title: String,
}

impl CiRun {
    /// Whether the run says main is broken: a failure, a timeout, or a workflow that never started.
    #[must_use]
    pub fn red(&self) -> bool {
        matches!(self.conclusion.as_str(), "failure" | "timed_out" | "startup_failure")
    }
}

/// What `gh run view --log-failed` says of a red run: the job and the step that failed, the
/// first failing test, and the first error line.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CiFailure {
    pub job: Option<String>,
    pub step: Option<String>,
    pub test: Option<String>,
    pub error: String,
}

/// Whether the CI of the project at `root` is due a read at `now`: never read, or an hour since.
#[must_use]
pub fn ci_due(root: &Path, now: u64) -> bool {
    let last = std::fs::read_to_string(root.join(CI_STAMP)).ok().and_then(|t| t.trim().parse::<u64>().ok());
    last.is_none_or(|last| now.saturating_sub(last) >= CI_EVERY)
}

/// The latest completed run of each workflow in a `gh run list --json` text, newest first as gh
/// lists them, in the order the workflows first appear; a run still going is passed over, so
/// the one before it speaks for its workflow.
#[must_use]
pub fn ci_latest(json: &str) -> Vec<CiRun> {
    let Ok(serde_json::Value::Array(runs)) = serde_json::from_str::<serde_json::Value>(json.trim()) else { return Vec::new() };
    let mut latest: Vec<CiRun> = Vec::new();
    for run in &runs {
        let text = |key: &str| run.get(key).and_then(serde_json::Value::as_str).unwrap_or_default().to_owned();
        let workflow = text("workflowName");
        if text("status") != "completed" || workflow.is_empty() || latest.iter().any(|r| r.workflow == workflow) {
            continue;
        }
        latest.push(CiRun {
            workflow,
            conclusion: text("conclusion"),
            commit: text("headSha"),
            id: run.get("databaseId").and_then(serde_json::Value::as_u64).unwrap_or_default(),
            url: text("url"),
            title: text("displayTitle"),
        });
    }
    latest
}

/// What a `gh run view --log-failed` text says failed. Each of its lines is the job, the step and
/// the timestamped output, tab-separated; the first line's job and step are the ones that failed.
#[must_use]
pub fn ci_failure(log: &str) -> CiFailure {
    let mut failure = CiFailure::default();
    let mut output = String::new();
    for line in log.lines() {
        let line = line.trim_start_matches('\u{feff}');
        let mut fields = line.splitn(3, '\t');
        let (Some(job), Some(step), Some(rest)) = (fields.next(), fields.next(), fields.next()) else { continue };
        if failure.job.is_none() {
            failure.job = Some(job.trim().to_owned());
            failure.step = Some(step.trim().to_owned());
        }
        // `2026-11-12T10:00:00.1234567Z text`
        let text = match rest.split_once(' ') {
            Some((stamp, text)) if stamp.ends_with('Z') && stamp.contains('T') => text,
            _ => rest,
        };
        output.push_str(&without_ansi(text));
        output.push('\n');
    }
    failure.test = failed_test_names(&output, false).into_iter().next().or_else(|| failed_test_names(&output, true).into_iter().next());
    failure.error = first_error(&output);
    failure
}

/// `text` without its terminal colour codes, which a CI log keeps.
fn without_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // `ESC [ … letter`
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// gh as the machine has it, run in `root` so it reads the project's remote: on PATH, else
/// where its installer puts it. `None` when there is none.
fn gh_of(root: &Path) -> Option<Gh> {
    let search: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    let exe = crate::hub::manifest::resolve("gh", &search).or_else(|| Some(PathBuf::from(GH_DEFAULT)).filter(|p| p.is_file()))?;
    let root = root.to_path_buf();
    let gh: Gh = Arc::new(move |args: &[&str]| {
        let mut command = std::process::Command::new(&exe);
        command.args(args).current_dir(&root).stdin(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // a read from a walk with no console opens no window of its own
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let out = command.output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    });
    Some(gh)
}

/// One gh call off the runtime's threads.
async fn gh_call(gh: &Gh, args: &[&str]) -> Option<String> {
    let gh = gh.clone();
    let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    tokio::task::spawn_blocking(move || gh(&args.iter().map(String::as_str).collect::<Vec<_>>())).await.ok().flatten()
}

/// Reads the project's CI on main when it is due at `now` ([`ci_due`]) and gh is on the machine
/// and logged in: [`read_ci_with`] with the machine's gh. Nothing read the CI before, and the
/// agent's own build went three days red (2026-11-12 to 11-15) unseen.
pub async fn read_ci(store: &Store, now: u64) -> Vec<String> {
    let root = workspace_of(store);
    if !ci_due(&root, now) {
        return Vec::new();
    }
    if let Some(gh) = gh_of(&root) {
        read_ci_with(store, now, gh).await
    } else {
        let _ = std::fs::create_dir_all(root.join("var"));
        let _ = std::fs::write(root.join(CI_STAMP), now.to_string());
        Vec::new()
    }
}

/// [`read_ci`] with gh named: the latest completed run of each workflow on main
/// (`gh run list --branch main --limit 5`), kept in [`crate::hub::CI_FILE`] for `rc status` and
/// the dashboard; a red one, which no green run of its workflow has superseded, files one
/// priority-1 card under the Look-back milestone naming the workflow, the job, the step and the
/// first failing test of `gh run view --log-failed`. One line for the read and one per card.
pub async fn read_ci_with(store: &Store, now: u64, gh: Gh) -> Vec<String> {
    let root = workspace_of(store);
    if !ci_due(&root, now) {
        return Vec::new();
    }
    let _ = std::fs::create_dir_all(root.join("var/runs"));
    let _ = std::fs::write(root.join(CI_STAMP), now.to_string());
    if gh_call(&gh, &["auth", "status"]).await.is_none() {
        return Vec::new();
    }
    // a project with no GitHub remote, or a network down, keeps the last read
    let Some(json) = gh_call(&gh, &["run", "list", "--branch", "main", "--limit", "5", "--json", CI_FIELDS]).await else {
        return Vec::new();
    };
    let runs = ci_latest(&json);
    if runs.is_empty() {
        return Vec::new();
    }
    let mut text = String::new();
    for run in &runs {
        let _ = writeln!(text, "{}\t{}\t{}\t{}\t{}\t{now}", run.workflow, run.conclusion, run.commit, run.id, run.url);
    }
    let _ = std::fs::write(root.join(crate::hub::CI_FILE), text);
    let mut lines = vec![format!("ci on main: {}", crate::hub::ci_state(&root).map(|s| s.line).unwrap_or_default())];
    let mut filed: Vec<(String, String)> = std::fs::read_to_string(root.join(CI_FILED))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.split_once('\t').map(|(w, c)| (w.to_owned(), c.to_owned())))
        .collect();
    filed.retain(|(workflow, _)| !runs.iter().any(|run| &run.workflow == workflow && !run.red()));
    for run in runs.iter().filter(|run| run.red()) {
        if filed.iter().any(|(workflow, _)| *workflow == run.workflow) {
            continue;
        }
        let log = gh_call(&gh, &["run", "view", &run.id.to_string(), "--log-failed"]).await.unwrap_or_default();
        let failure = ci_failure(&log);
        match file_ci_failure(store, run, &failure).await {
            Ok(id) => {
                let line = format!("#{id} filed: the workflow `{}` is {} on main", run.workflow, run.conclusion);
                crate::pair::event(&root, &line);
                lines.push(line);
                filed.push((run.workflow.clone(), id.to_string()));
            }
            Err(e) => lines.push(format!("the card for the workflow `{}` could not be filed: {e:#}", run.workflow)),
        }
    }
    let mut text = String::new();
    for (workflow, card) in &filed {
        text.push_str(workflow);
        text.push('\t');
        text.push_str(card);
        text.push('\n');
    }
    let _ = std::fs::write(root.join(CI_FILED), text);
    lines
}

/// Files the card of a workflow red on main: priority 1, under the Look-back milestone, its
/// check the first failing test when the log names one.
async fn file_ci_failure(store: &Store, run: &CiRun, failure: &CiFailure) -> anyhow::Result<i64> {
    let parent = crate::plan::home::ensure_milestone(store).await?;
    let short: String = run.commit.chars().take(10).collect();
    let unnamed = "none the log names";
    let job = failure.job.as_deref().unwrap_or(unnamed);
    let step = failure.step.as_deref().unwrap_or(unnamed);
    let test = failure.test.as_deref().unwrap_or(unnamed);
    let error = if failure.error.is_empty() { unnamed } else { failure.error.as_str() };
    let objective = format!(
        "The workflow {}'s latest completed run on main ended {} at {short} ({}), and no green run has superseded it: {}\n\nJob: {job}\nStep: {step}\nFirst failing test: {test}\nFirst error: {error}",
        run.workflow, run.conclusion, run.title, run.url
    );
    // a cargo test's name is a filter of the workspace's tests; a timeout names none
    let cargo = workspace_of(store).join("Cargo.toml").is_file();
    let acceptance = match failure.test.as_deref() {
        Some(test) if cargo && !test.contains(' ') => format!("`cargo test --workspace {test}` passes"),
        _ => format!("the workflow {}'s next run on main ends green within its time limit", run.workflow),
    };
    let title = match failure.test.as_deref() {
        Some(test) => format!("CI {} is {} on main: {test}", run.workflow, run.conclusion),
        None => format!("CI {} is {} on main in {step}", run.workflow, run.conclusion),
    };
    let card = Card { objective: Some(&objective), acceptance: Some(&acceptance), ..Card::new() };
    let id = store.insert_card(Some(parent), NodeKind::Task, &title, &card).await?;
    store.set_priority(id, Some(1)).await?;
    Ok(id)
}

/// Gives `session` a fresh build budget in `tree`, for a rerun whose prompt is a lint fix: the
/// builds of the run before it were spent on the card's change, and the fix runs the checks.
pub fn reset_builds(tree: &Path, session: &str) {
    let name: String = session.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    let _ = std::fs::remove_file(tree.join("var").join("builds").join(name));
}

/// Whether a `cargo test` or gradle `test` command names test filters: a run of the failing
/// tests by name, not the suite. A cargo filter is a positional word after `test` (before or
/// after `--`) that is not the value of a flag; gradle's is `--tests`.
#[must_use]
pub fn names_tests(command: &str) -> bool {
    // flags of cargo test that take the next word as their value
    const VALUED: [&str; 11] =
        ["-p", "--package", "--features", "-F", "--target", "--manifest-path", "--test", "--bin", "--example", "-j", "--jobs"];
    let words: Vec<&str> = command.split_whitespace().collect();
    if words.iter().any(|w| *w == "--tests" || w.starts_with("--tests=")) {
        return true;
    }
    let Some(at) = words.windows(2).position(|w| w[0].ends_with("cargo") && w[1] == "test") else {
        return false;
    };
    let mut rest = words[at + 2..].iter();
    let mut after_dashes = false;
    while let Some(word) = rest.next() {
        // the command ends at a pipe or a chain, and a redirect (the `2>&1 | tail` a gotcha
        // appends) is the shell's, not a filter
        if matches!(*word, "|" | "||" | "&&") || word.ends_with(';') {
            break;
        }
        if word.contains(['>', '<']) {
            // a bare `>` or `2>` takes the next word as its target
            if word.ends_with(['>', '<']) {
                rest.next();
            }
            continue;
        }
        if *word == "--" {
            after_dashes = true;
        } else if !after_dashes && VALUED.contains(word) {
            rest.next();
        } else if !word.starts_with('-') {
            return true;
        }
    }
    false
}

/// Runs a build command in `dir` on one of its project's build slots, for `rc hold`: waits for
/// a slot, says so once on stderr, then runs the command through `shell` (`bash`, `sh` or
/// `powershell`) with the terminal's own streams, and answers with the command's exit code. The
/// hook rewrites a worker's gradle and cargo commands to this, so a project's six workers
/// compile two at a time. A slot that cannot be locked at all is nobody's failure: the command
/// runs unguarded.
#[must_use]
pub fn hold(dir: &Path, shell: &str, command: &str) -> i32 {
    let mut said = false;
    let started = std::time::Instant::now();
    let _slot = loop {
        match crate::hub::try_build_slot(dir) {
            Ok(Some(guard)) => break Some(guard),
            Ok(None) => {}
            Err(e) => {
                eprintln!("rc hold: the command runs without a worker slot: {e:#}");
                break None;
            }
        }
        if started.elapsed() >= HOLD_WAIT {
            eprintln!("rc hold: no worker slot came free in {} minutes; the command runs without one", HOLD_WAIT.as_secs() / 60);
            break None;
        }
        if !said {
            said = true;
            eprintln!(
                "rc hold: waiting for a worker slot, {} of the machine's {} are running",
                crate::hub::builds_running(),
                crate::hub::build_slot_count()
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    };
    let mut process = match shell {
        "powershell" | "pwsh" => {
            let mut p = std::process::Command::new(shell);
            p.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", command]);
            p
        }
        _ => {
            // `bash` on a Windows PATH may be WSL's, which has none of the worker's tools: the
            // one Git installs is the Bash tool's own
            let git_bash = Path::new(r"C:\Program Files\Git\bin\bash.exe");
            let program = if shell == "bash" && git_bash.exists() { git_bash.as_os_str().to_owned() } else { shell.into() };
            let mut p = std::process::Command::new(program);
            p.args(["-c", command]);
            p
        }
    };
    let mut child = match process.current_dir(dir).spawn() {
        Ok(child) => child,
        Err(e) => {
            eprintln!("rc hold: cannot start {shell}: {e}");
            return 127;
        }
    };
    // the build runs on its share of the builders' CPU, following the number of builds
    share_cpu_among_builds();
    #[cfg(windows)]
    let _share = {
        use std::os::windows::io::AsRawHandle;
        ra_tools::cpu::attach(child.as_raw_handle())
    };
    child.wait().map_or(1, |status| status.code().unwrap_or(1))
}

/// How long `rc wait` waits unless told otherwise: under a worker's 600-second turn, so the
/// turn ends with an answer rather than a kill.
pub const WAIT_LIMIT: std::time::Duration = std::time::Duration::from_mins(9);

/// How long a file `rc wait` watches must hold still before its job counts as done.
const WAIT_SETTLE: std::time::Duration = std::time::Duration::from_secs(10);

/// The lines of the file `rc wait` prints once it settles.
const WAIT_TAIL_LINES: usize = 40;

/// Waits on a long job of the worker's own, for `rc wait`: a pid until the process is gone, a
/// file until it exists and has not changed for [`WAIT_SETTLE`], whose last lines it then
/// prints. The one sanctioned wait: a `sleep` loop is refused, and a background shell the
/// session outlives is no result (#159). Answers 0 when the job ended within `limit`, 124 when
/// the limit ran out first, as `timeout` does.
#[must_use]
pub fn wait(dir: &Path, target: &str, limit: std::time::Duration) -> i32 {
    let started = std::time::Instant::now();
    let poll = std::time::Duration::from_millis(500);
    if let Ok(pid) = target.trim().parse::<u32>() {
        while crate::hub::pid_alive(pid) {
            if started.elapsed() >= limit {
                eprintln!("rc wait: pid {pid} still runs after {} s", limit.as_secs());
                return 124;
            }
            std::thread::sleep(poll);
        }
        println!("rc wait: pid {pid} has ended");
        return 0;
    }
    let path = dir.join(target);
    let seen = |path: &Path| std::fs::metadata(path).ok().map(|m| (m.len(), m.modified().ok()));
    let mut last = seen(&path);
    let mut still = std::time::Instant::now();
    let code = loop {
        if started.elapsed() >= limit {
            eprintln!("rc wait: {target} still changes after {} s", limit.as_secs());
            break 124;
        }
        std::thread::sleep(poll);
        let now = seen(&path);
        if now != last {
            last = now;
            still = std::time::Instant::now();
        } else if last.is_some_and(|(len, _)| len > 0) && still.elapsed() >= WAIT_SETTLE {
            // an empty file is a job not yet writing (a compile), not a finished one
            break 0;
        }
    };
    if let Ok(text) = std::fs::read_to_string(&path) {
        println!("{}", last_lines(&text, WAIT_TAIL_LINES));
    }
    code
}

/// The share of the processor every build on the machine runs at together: one build may use
/// it all, `n` builds a `1/n` part each. The person's rule of 2026-11-12: "1 builder 90%, 3 at
/// 30% each".
pub const BUILD_CPU: u32 = 9000;

/// Sets this process's builds to run on their share of [`BUILD_CPU`]: the whole budget split
/// evenly among the builds holding a slot right now, read again as they come and go.
pub fn share_cpu_among_builds() {
    ra_tools::cpu::set_rate(Box::new(|| {
        let running = u32::try_from(crate::hub::builds_running()).unwrap_or(1).max(1);
        BUILD_CPU / running
    }));
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

/// Holds the run while the plan's week, across every project on the machine, is at or past
/// `mark`: says `line` once, on the progress channel and the paired feed, and reads the limits
/// again every minute until the week resets.
async fn wait_for_week(root: &Path, mark: f64, line: &str) {
    tracing::info!(target: crate::trace::PROGRESS, "  {line}");
    crate::pair::event(root, line);
    while plan_week(&plan_readings(root), mark, unix_now()).is_some() {
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

/// The cards a restart orphaned, back on the queue: every run in the activity log that started,
/// never ended and whose process is gone ([`crate::pair::orphaned`]) has its card set from
/// `in_progress` back to `defined` under [`Source::Run`], its session kept, so the next run
/// resumes it as a rerun would; a card already past `in_progress` is left as it is. One feed
/// line names them all, `re-queued 3 orphaned run(s): #179 #181 #184`, and is the line
/// returned; none when no card went back. Before this the history showed the person
/// resetting four or five cards by hand after each walk restart (2026-11-12).
pub async fn requeue_orphans(store: &Store, root: &Path) -> Option<String> {
    requeue_orphans_with(store, root, crate::hub::pid_alive).await
}

/// [`requeue_orphans`] with the liveness check given, for the tests.
pub async fn requeue_orphans_with(store: &Store, root: &Path, alive: impl Fn(u32) -> bool) -> Option<String> {
    let orphaned = crate::pair::orphaned(root, alive);
    if orphaned.is_empty() {
        return None;
    }
    let mut requeued = Vec::new();
    let mut left = Vec::new();
    for id in orphaned {
        match store.read(id).await {
            Ok(node) if node.state == TaskState::InProgress => match store.set_state_by(id, TaskState::Defined, Source::Run).await {
                Ok(()) => requeued.push(format!("#{id}")),
                Err(e) => left.push(format!("#{id} (not written: {e:#})")),
            },
            Ok(node) => left.push(format!("#{id} ({})", node.state)),
            Err(_) => left.push(format!("#{id} (no such card)")),
        }
    }
    // no line when nothing went back: `live_start` closes an open start on a `re-queued` line,
    // and a window coming up beside a live run of another process must not read it idle
    if requeued.is_empty() {
        return None;
    }
    let mut text = format!("re-queued {} orphaned run(s): {}", requeued.len(), requeued.join(" "));
    if !left.is_empty() {
        let _ = write!(text, " · left as they are: {}", left.join(" "));
    }
    crate::pair::event(root, &text);
    Some(text)
}

/// A run of an earlier window the window adopted ([`crate::pair::window_up`]): waits, asking
/// `alive` every `poll`, while process `pid` of card `id` lives, then takes its end as the
/// run's own. A run that wrote its end is closed already and nothing more is written; one that
/// died without it is an orphan, and its end and re-queue are written once, through
/// [`requeue_orphans`], whose line is returned.
pub async fn adopt(
    store: &Store,
    root: &Path,
    id: i64,
    pid: u32,
    alive: impl Fn(u32) -> bool,
    poll: std::time::Duration,
) -> Option<String> {
    while alive(pid) {
        tokio::time::sleep(poll).await;
    }
    tracing::debug!("adopted run #{id} of process {pid} ended");
    // only its own run is the adoption's to close: another adopted run may still go, and the
    // runs no window adopted were judged when the window came up
    requeue_orphans_with(store, root, |other| other != pid).await
}

/// Runs the ready cards one after another, reading readiness again after each, until none is
/// ready or a run fails. A card a run left needing revision is rerun by the walk on its session,
/// after a failed check twice and after a rejection once ([`CHECK_RETRIES`], [`REVIEW_RETRIES`]);
/// past that it waits for the person, and with `stop_on_revision` it stops the batch too.
/// When none is ready and an active gambit says `plan-next`, the planner fills the next empty
/// milestone first ([`crate::commands::run::plan_next`]) and the walk goes on with its cards.
/// Each run is its own bubble and its own handoff, through [`run`], in the card's worktree as a
/// parallel walk's are ([`walk_runner`]): a landing commits the card's work and nothing of the
/// project's tree (#202's commit swept in a pairing session's edits, 2026-11-12). A failed run ends the batch
/// with its reason on the last line rather than as an error, so the runs before it keep their
/// lines.
///
/// # Errors
/// When the roadmap cannot be read between runs.
#[allow(clippy::too_many_lines)]
pub async fn run_all(store: &Store, worker: &Worker, stop_on_revision: bool) -> anyhow::Result<Vec<String>> {
    share_cpu_among_builds();
    // a walk that dies takes its workers with it
    ra_tools::cpu::own_children();
    probe_the_machine(&workspace_of(store)).await;
    let mut lines = Vec::new();
    let mut ran = 0;
    // the card the last run continued and how many runs in a row it has had: a card the
    // project's turn cap keeps ending is picked again fresh, up to [`RESTARTS`] times
    let mut again: Option<(i64, usize)> = None;
    // whether the planner filled the roadmap since the last run: it plans once between runs,
    // so a plan that leaves no card ready ends the walk rather than plan the next milestone too
    let mut planned = false;
    // the card the walk takes up again by itself, and per card the check failures and the
    // rejections this walk has seen: a person re-queued those by hand before (#103, #47, #134)
    let mut retry: Option<i64> = None;
    // the cards past their reruns in this walk
    let mut parked: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let mut sent: std::collections::HashMap<i64, (usize, usize)> = std::collections::HashMap::new();
    // the cards held for a missing prerequisite, and the reason last told
    let mut held: std::collections::HashMap<i64, String> = std::collections::HashMap::new();
    // one walk per project: the lock is held for the whole of it, and a walk moving to a newer
    // install hands it to its successor as it exits
    let lock = match take_walk_lock(&workspace_of(store)) {
        Some(lock) => Some(lock),
        None => handed_walk_lock(&workspace_of(store)).await,
    };
    let Some(_lock) = lock else {
        anyhow::bail!("a walk is already running in this project; `rc walk --stop` ends it after its current card")
    };
    note_walk_pid(&workspace_of(store));
    // what the install said at the start: a change since is a newer rc to move to
    let seen = worker.install.as_deref().and_then(crate::hub::installed);
    // set when the move failed: the walk finishes on its own rc rather than try at every card
    let mut stays = false;
    // the other projects see the walk from its start
    let root = workspace_of(store);
    write_digest(store, &root, Some(format!("walk · pid {} · started {}", std::process::id(), clock(unix_now()))), None, None).await;
    // the cards a killed walk left in progress go back on the queue before this one reads it
    lines.extend(requeue_orphans(store, &root).await);
    // the planned cards done a day ago are read back first: the day since them against the
    // day before, so the plan's claims are judged before the walk spends the next day
    lines.extend(crate::commands::plan::read_back(store, &root, worker).await);
    // the cards a review off the form held are judged again before any card runs
    lines.extend(review_held(store, worker).await);
    loop {
        // the cards that wait on a red main are ready again once main moves
        lines.extend(requeue_red_main(store, &root).await);
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
        // a rerun the walk queued goes first, while the card still waits on it: a person who
        // took the card meanwhile has the say
        // and not while it waits on cards a send-back split off
        let retried = retry.take().filter(|id| {
            roadmap.nodes.iter().any(|n| {
                n.id == *id
                    && n.state == TaskState::NeedsRevision
                    && n.depends_on.iter().all(|dep| roadmap.nodes.iter().any(|d| d.id == *dep && d.state == TaskState::Accepted))
            })
        });
        // a card past its reruns in this walk waits for the next walk: the queue counts a card
        // sent back as ready, and without this the one-at-a-time walk took the same card again;
        // a card whose prerequisites are missing waits too, named
        let mut picked = None;
        lines.extend(say_review_waits(&root, &roadmap.nodes, &std::collections::HashSet::new(), &mut held));
        if retried.is_none() {
            for node in ready_cards(&roadmap.nodes, &roadmap.gambits, tight).into_iter().filter(|n| !parked.contains(&n.id)) {
                match prerequisites_missing(&root, node).await {
                    None => {
                        picked = Some(node.id);
                        break;
                    }
                    Some(why) => lines.extend(say_held(&root, node, &why, &mut held)),
                }
            }
        }
        let Some(id) = retried.or(picked) else {
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
        // an absolute card is continued as often as it takes: only a full stop ends it
        let absolute = store.read(id).await.is_ok_and(|n| n.is_absolute());
        if runs > RESTARTS && !absolute {
            done(
                &mut lines,
                format!("ran {ran} card(s) · stopped: #{id} continued {RESTARTS} times in a row; raise [worker] turns or split the card"),
            )
            .await;
            return Ok(lines);
        }
        // a card the context cap ended three times in a row is split, not continued a fourth
        // time: the planner proposes the remaining work as cards, the card is blocked with it
        if crate::plan::split::due(&root, &store.read(id).await?) {
            lines.push(crate::plan::split::split(store, &root, id, worker).await?);
            parked.insert(id);
            continue;
        }
        // the ledger's mark before the run: the rows after it are the run's
        let mark = match worker.ledger() {
            Some(ledger) => ledger.last_id().await.ok(),
            None => None,
        };
        let runner = match walk_runner(store, &root, id, worker).await {
            Ok((runner, caught_up)) => {
                lines.extend(caught_up);
                runner
            }
            Err(error) => {
                done(&mut lines, format!("ran {ran} card(s) · stopped: the worktree of #{id} could not be made: {error:#}")).await;
                return Ok(lines);
            }
        };
        match run_picked(store, id, &runner, tight && retried.is_none()).await {
            Ok(run_lines) => lines.extend(run_lines),
            Err(error) => {
                // the plan's own limit is the one failure that ends the week's work: the walk
                // waits out the reset and takes the card up again; any other failure (a steer, a
                // route, a model error) is the card's, set aside while the walk goes on (both
                // walks died on one card's steer, 2026-11-14)
                if plan_spent(&root) {
                    let line = format!("the run of #{id} failed on the plan's limit: {error:#}; the walk waits for the reset");
                    // the tests never wait out a week
                    if cfg!(test) {
                        done(&mut lines, format!("ran {ran} card(s) · stopped: {line}")).await;
                        return Ok(lines);
                    }
                    lines.push(line.clone());
                    wait_for_week(&root, 1.0, &line).await;
                    again = None;
                    continue;
                }
                lines.push(set_aside(&root, id, &error));
                parked.insert(id);
                continue;
            }
        }
        ran += 1;
        planned = false;
        // the card's claimed saving is judged on the run's own rows, as it lands
        if let (Some(mark), Some(ledger), Ok(node)) = (mark, worker.ledger(), store.read(id).await) {
            lines.extend(crate::retro::landed(&root, &node, ledger, mark).await);
        }
        let mut node = store.read(id).await?;
        // a card the reviewer accepted lands now: committed, proved and cleared; one the landing
        // sent back is counted below as any card sent back
        if node.state == TaskState::Implemented {
            lines.extend(land_or_say(store, id, &runner).await);
            node = store.read(id).await?;
        } else {
            // the walk runs one card at a time: the git lock is its own
            let lock = tokio::sync::Mutex::new(());
            lines.extend(checkpoint_and_prove(&root, &node, &runner, lock.lock().await).await);
        }
        if node.state == TaskState::NeedsRevision {
            // the first rejection and the first two check failures are rerun on the card's
            // session, the verdict and the failing check as the prompt; past that it is the person's
            let rejected = rejected_by_review(&root, id);
            let (checks, rejections) = sent.entry(id).or_default();
            let (count, allowed, why) = if rejected {
                *rejections += 1;
                (*rejections, REVIEW_RETRIES, format!("the reviewer sent it back {} time(s)", *rejections))
            } else {
                *checks += 1;
                (*checks, CHECK_RETRIES, format!("its check failed {} time(s)", *checks))
            };
            // an absolute card goes back to work however often it is sent back, while its runs
            // change files: one that changed nothing is stuck, and the counts hold it again
            let moved = ra_context::TaskContext::open(&root, id).handoffs.last().is_some_and(|h| !h.changed.is_empty());
            let rerun = (count <= allowed || (node.is_absolute() && moved)).then_some((count, allowed.max(count)));
            lines.push(crate::pair::sent_back(&root, id, &node.title, &why, rerun));
            let blocked = if rejected { crate::hub::block_after_send_backs(store, &root, id).await } else { None };
            let rerun = rerun.filter(|_| blocked.is_none());
            lines.extend(blocked);
            if rerun.is_some() {
                retry = Some(id);
            } else {
                parked.insert(id);
            }
            if rerun.is_none() && stop_on_revision {
                done(&mut lines, format!("ran {ran} card(s) · stopped: #{id} needs revision")).await;
                return Ok(lines);
            }
        }
        // `stop` from the console or the window: the walk ends after the run it was in
        if take_stop(&root) {
            done(&mut lines, format!("ran {ran} card(s) · stopped after #{id}, as asked")).await;
            return Ok(lines);
        }
        // a newer rc installed: it takes the walk up from the next card, so `rc install` is
        // enough and no run in flight is cut; a card being rerun keeps the walk that counts its
        // retries
        if !stays
            && retry.is_none()
            && let Some(dir) = &worker.install
            && let Some(version) = crate::hub::moved_to(crate::VERSION, seen.as_deref(), crate::hub::installed(dir).as_deref())
        {
            match (worker.relaunch)(dir, &root) {
                Ok(pid) => {
                    done(&mut lines, format!("moved to {version} · ran {ran} card(s) · pid {pid}")).await;
                    return Ok(lines);
                }
                Err(error) => {
                    // the walk stays on its own rc; the line says why, and it does not try again
                    lines.push(format!("the walk stays on {}: {error:#}", crate::VERSION));
                    crate::pair::event(&root, &format!("walk stays on {}: {error:#}", crate::VERSION));
                    stays = true;
                }
            }
        }
    }
}

/// Whether the plan's week, as the latest reading on the machine has it, is used up: the plan's
/// own limit, the one that ends a walk's week rather than one card's run.
fn plan_spent(root: &Path) -> bool {
    plan_week(&plan_readings(root), 1.0, unix_now()).is_some()
}

/// Sets card `id` aside for this walk after its run failed with `error`: the reason on the
/// progress channel and the paired feed, and the line for the walk's output.
fn set_aside(root: &Path, id: i64, error: &anyhow::Error) -> String {
    let text = format!("set aside #{id}: its run failed: {}; the walk goes on with the next ready card", first_line(&format!("{error:#}")));
    tracing::warn!(target: crate::trace::PROGRESS, "{text}");
    crate::pair::event(root, &text);
    text
}

/// How many runs in a row a walk gives one card before it stops: a card the project's turn cap
/// ends this often is too large for the cap.
const RESTARTS: usize = 5;

/// The worker a one-at-a-time walk runs card `id` with: in `var/worktrees/<id>` on
/// `card/<id>`, the project's branch merged in first, so the run, its checkpoint and its
/// landing touch nothing the person or a pairing session left uncommitted in the project's
/// tree; the lines the catch-up said. A project that is no git checkout, or one with no
/// commit to branch from, has no worktree to make, and its cards run in its own tree.
///
/// # Errors
/// When the card is not there, or git cannot add the worktree.
async fn walk_runner(store: &Store, root: &Path, id: i64, worker: &Worker) -> anyhow::Result<(Worker, Vec<String>)> {
    if !root.join(".git").exists() || git(root, &["rev-parse", "--verify", "--quiet", "HEAD"]).await.is_err() {
        return Ok((worker.clone(), Vec::new()));
    }
    let node = store.read(id).await?;
    let (path, branch) = worktree_add(root, id).await?;
    let runner = worker.clone().with_worktree(path.clone(), branch);
    let mut lines = Vec::new();
    // a clean merge is proven before the worker gets the tree; a failure is the run's first job
    if let Some((line, merge_base)) = catch_up_worktree(root, &path, &node).await {
        lines.push(line);
        relocate_handoff_points(root, &path, id);
        if let Some(merge_base) = merge_base {
            lines.extend(prove_tree(root, &path, &node, &runner, &merge_base, "the merge of the project's branch").await);
        }
    }
    Ok((runner, lines))
}

/// After a catch-up merge, the last handoff's warnings are found again by their quoted text in
/// the merged tree and their line numbers rewritten; a point whose text is gone says `resolved?`.
fn relocate_handoff_points(root: &Path, tree: &Path, id: i64) {
    let mut task = ra_context::TaskContext::open(root, id);
    let Some(last) = task.handoffs.last_mut() else { return };
    if last.warnings.is_empty() {
        return;
    }
    let moved = crate::commands::review::points::relocate_all(&last.warnings, |file| std::fs::read_to_string(tree.join(file)).ok());
    if moved != last.warnings {
        last.warnings = moved;
        if let Err(e) = task.save(root) {
            tracing::warn!("the context of #{id} was not saved: {e:#}");
        }
    }
}

/// How many times a walk reruns a card whose check (or lint) failed before it leaves the card
/// to the person: the third failure waits.
const CHECK_RETRIES: usize = 2;
/// How many times a walk reruns a card the reviewer sent back: the second rejection waits.
const REVIEW_RETRIES: usize = 1;

/// Whether the reviewer sent card `id` back, read off the last handoff's verdict; a card that
/// needs revision without one failed its check or its lint, which the reviewer never saw.
fn rejected_by_review(root: &Path, id: i64) -> bool {
    ra_context::TaskContext::open(root, id)
        .handoffs
        .last()
        .and_then(|last| last.verdict.as_deref())
        .is_some_and(|verdict| verdict.starts_with("needs revision"))
}

/// The times the reviewer sent card `id` back, counted over its handoffs as
/// [`rejected_by_review`] reads the last: what a route rule's `rejections` is held against.
#[must_use]
pub fn rejections(root: &Path, id: i64) -> usize {
    ra_context::TaskContext::open(root, id)
        .handoffs
        .iter()
        .filter(|handoff| handoff.verdict.as_deref().is_some_and(|verdict| verdict.starts_with("needs revision")))
        .count()
}

/// The mark a review off the form twice leaves on the last handoff's verdict, before the
/// reviewer's model.
const NO_VERDICT: &str = "no verdict by";

/// The reviewer whose answers on card `id` were off the form twice, when its last handoff
/// carries [`NO_VERDICT`].
fn held_without_verdict(root: &Path, id: i64) -> Option<String> {
    let task = ra_context::TaskContext::open(root, id);
    let verdict = task.handoffs.last()?.verdict.as_deref()?;
    verdict.strip_prefix(NO_VERDICT).map(|model| model.trim().to_owned()).filter(|model| !model.is_empty())
}

/// The model one rung above `model` on `ladder`: `model` itself at the top or off the ladder.
fn rung_above(ladder: &crate::router::Ladder, model: &str) -> String {
    let own = resolve(model);
    ladder.models.iter().position(|rung| resolve(rung) == own).and_then(|at| ladder.models.get(at + 1)).map_or(own, |above| resolve(above))
}

/// The walk's pass over the cards held in review for want of a verdict ([`NO_VERDICT`]): each
/// is reviewed again, one rung above the reviewer that wrote off the form, and one it accepts
/// lands, so a held card never waits on a person. The lines of the reviews and the landings.
async fn review_held(store: &Store, worker: &Worker) -> Vec<String> {
    let root = workspace_of(store);
    let Ok(nodes) = store.all().await else { return Vec::new() };
    let mut lines = Vec::new();
    for node in nodes {
        if node.state != TaskState::Review || held_without_verdict(&root, node.id).is_none() {
            continue;
        }
        let span = tracing::info_span!(target: crate::trace::PROGRESS, crate::trace::CARD_SPAN, id = node.id);
        let id = node.id;
        async {
            lines.extend(review_after_run(store, id, worker).await);
            if store.read(id).await.is_ok_and(|n| n.state == TaskState::Implemented) {
                lines.extend(land_or_say(store, id, worker).await);
            }
        }
        .instrument(span)
        .await;
    }
    lines
}

/// The file a `stop` leaves for a walk, which reads it between runs.
const WALK_STOP: &str = "var/walk.stop";
/// The file `rc walk` leaves for the window, which starts the walk as its own process.
const WALK_START: &str = "var/walk.start";
/// The pid of the walk that last took the lock, for a `stop` to name.
const WALK_PID: &str = "var/walk.pid";
/// The file a running walk holds open for exclusive use, so a second one does not start.
const WALK_LOCK: &str = "var/walk.lock";

/// The lock a running walk holds; dropping it lets the next walk start.
pub struct WalkLock(#[allow(dead_code)] std::fs::File, PathBuf);

impl Drop for WalkLock {
    /// The pid note goes before the lock does, so a question about the walk never finds a note
    /// of a walk that is over.
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.1);
    }
}

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
        Some(walk_lock_noted(root, file))
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
        Some(walk_lock_noted(root, file))
    }
}

/// The lock with the holder's pid noted beside it, for [`walk_running`].
fn walk_lock_noted(root: &Path, file: std::fs::File) -> WalkLock {
    note_walk_pid(root);
    WalkLock(file, root.join(WALK_PID))
}

/// Whether a walk is running in the project right now: asked of the pid the holder noted, never
/// by taking the lock, which a walk starting in that instant would find held.
#[must_use]
pub fn walk_running(root: &Path) -> bool {
    std::fs::read_to_string(root.join(WALK_PID)).ok().and_then(|t| t.trim().parse::<u32>().ok()).is_some_and(crate::hub::pid_alive)
}

/// Asks the window that has the project to start the walk: leaves the file its tick reads,
/// with the number of cards to run at once when more than one.
#[must_use]
pub fn request_walk(root: &Path, parallel: Option<u32>) -> String {
    let _ = std::fs::create_dir_all(root.join("var"));
    let text = parallel.filter(|n| *n > 1).map(|n| n.to_string()).unwrap_or_default();
    match std::fs::write(root.join(WALK_START), text) {
        Ok(()) => match parallel.filter(|n| *n > 1) {
            Some(n) => {
                format!("the window starts the walk, {n} cards at once in worktrees; they land in the project's tab and in var/pair.log")
            }
            None => "the window starts the walk; its cards land in the project's tab and in var/pair.log".to_owned(),
        },
        Err(e) => format!("cannot ask the window to start the walk: {e}"),
    }
}

/// Whether `rc walk` asked for a walk, and how many cards at once when more than one; the
/// request is consumed.
#[must_use]
pub fn walk_requested(root: &Path) -> Option<Option<u32>> {
    let path = root.join(WALK_START);
    let text = std::fs::read_to_string(&path).ok()?;
    std::fs::remove_file(path).ok()?;
    Some(text.trim().parse::<u32>().ok().filter(|n| *n > 1))
}

/// Starts the walk as a process of its own in `root`, detached from whoever asked: the
/// installed `rc run --all`, or the one beside this binary, `--parallel N` when more than one
/// card runs at once. Returns the process id.
///
/// # Errors
/// When a walk is running already, or `rc` cannot start.
pub fn spawn_walk(root: &Path, debug: bool, parallel: Option<u32>) -> anyhow::Result<u32> {
    anyhow::ensure!(!walk_running(root), "a walk is already running in this project");
    let rc = std::env::current_exe()?.with_file_name(format!("rc{}", std::env::consts::EXE_SUFFIX));
    let mut command = std::process::Command::new(&rc);
    command.args(["run", "--all"]).current_dir(root);
    // every walk is the parallel one, one runner included: it works in worktrees and reads its
    // runners again as it goes, so the dashboard's `+` on a one-runner project takes at once
    // (a one-at-a-time walk never read the number again, 2026-11-12)
    command.args(["--parallel", &parallel.unwrap_or(1).max(1).to_string()]);
    if !debug {
        command.arg("--no-debug");
    }
    crate::hub::detach(&mut command);
    // the walk's own complaints and panics land in var/walk.log: detached, it has no console,
    // and a walk that died at its start left no word of why (2026-11-11)
    let _ = std::fs::create_dir_all(root.join("var"));
    if let Ok(log) = std::fs::OpenOptions::new().create(true).append(true).open(root.join(WALK_LOG)) {
        command.stderr(log);
    }
    let child = command.spawn().map_err(|e| anyhow::anyhow!("cannot start {}: {e}", rc.display()))?;
    Ok(child.id())
}

/// The file a detached walk writes its stderr to: what it printed and why it died.
const WALK_LOG: &str = "var/walk.log";

/// How often a parallel walk looks again while its runs go: the roadmap, its runners and the
/// pool. Three seconds, so the dashboard's `+` on runners or workers acts within a moment (it
/// took up to half a minute, the person: "increasing runners on a project is not responsive").
const WALK_TICK: std::time::Duration = std::time::Duration::from_secs(3);

/// The variable a walk moving to a newer install leaves its successor: its own pid, whose walk
/// lock the successor waits for.
const WALK_FROM_VAR: &str = "RHIPPER_WALK_FROM";

/// How long a successor waits for the walk it takes over to let the lock go.
const HANDOVER: std::time::Duration = std::time::Duration::from_secs(120);

/// The walk lock of the walk this process takes over, when it was started by one moving to a
/// newer install: taken as that walk exits, so the project has one walk lock throughout.
async fn handed_walk_lock(root: &Path) -> Option<WalkLock> {
    let from = std::env::var(WALK_FROM_VAR).ok()?.parse::<u32>().ok()?;
    let start = std::time::Instant::now();
    while start.elapsed() < HANDOVER {
        if let Some(lock) = take_walk_lock(root) {
            return Some(lock);
        }
        if !crate::hub::pid_alive(from) {
            // gone and the lock still held: another walk has it
            return take_walk_lock(root);
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    None
}

/// Starts this walk again on the `rc` installed in `dir`, in `root`, with this process's
/// arguments, detached as [`spawn_walk`] starts one; the new walk takes the lock as this one
/// exits. Returns the process id.
///
/// # Errors
/// When the installed `rc` cannot start.
pub fn relaunch(dir: &Path, root: &Path) -> anyhow::Result<u32> {
    let rc = dir.join(format!("rc{}", std::env::consts::EXE_SUFFIX));
    let mut command = std::process::Command::new(&rc);
    command.args(std::env::args_os().skip(1)).current_dir(root).env(WALK_FROM_VAR, std::process::id().to_string());
    crate::hub::detach(&mut command);
    if let Ok(log) = std::fs::OpenOptions::new().create(true).append(true).open(root.join(WALK_LOG)) {
        command.stderr(log);
    }
    let child = command.spawn().map_err(|e| anyhow::anyhow!("cannot start {}: {e}", rc.display()))?;
    Ok(child.id())
}

/// The feed's line when a parallel walk sees `version` installed with `in_flight` runs going:
/// it starts no new card and hands over to the new rc once they end.
#[must_use]
pub fn handover_line(version: &str, in_flight: usize) -> String {
    if in_flight == 0 {
        format!("walk moving to {version}: it starts no new card and hands over now")
    } else {
        format!("walk moving to {version}: it starts no new card and hands over once its {in_flight} run(s) in flight end")
    }
}

/// Records the running walk's pid where a `stop` can read it: the walk lock cannot be read
/// while it is held.
fn note_walk_pid(root: &Path) {
    let _ = std::fs::write(root.join(WALK_PID), std::process::id().to_string());
}

/// Whether a stop file is for this walk, taking it up. A stop carries the pid of the walk it is
/// for: one for another walk is not this one's, and one for a dead walk is removed; one with no
/// pid is for any walk.
fn take_stop(root: &Path) -> bool {
    let path = root.join(WALK_STOP);
    let Ok(text) = std::fs::read_to_string(&path) else { return false };
    match text.trim().parse::<u32>() {
        Ok(pid) if pid != std::process::id() => {
            if !crate::hub::pid_alive(pid) {
                let _ = std::fs::remove_file(&path);
            }
            false
        }
        _ => {
            let _ = std::fs::remove_file(&path);
            true
        }
    }
}

/// Asks the walk to stop after the task it is in: leaves the file it reads between runs, with
/// the pid of the walk it is for. With no walk running nothing is left behind, so no later walk
/// stops after one card for it.
#[must_use]
pub fn stop_walk(root: &Path) -> String {
    let path = root.join(WALK_STOP);
    let _ = std::fs::create_dir_all(root.join("var"));
    let pid = std::fs::read_to_string(root.join(WALK_PID)).ok().and_then(|t| t.trim().parse::<u32>().ok());
    if let Some(pid) = pid
        && !crate::hub::pid_alive(pid)
    {
        let _ = std::fs::remove_file(&path);
        return "no walk is running here, so there is nothing to stop".to_owned();
    }
    match std::fs::write(&path, pid.map(|p| p.to_string()).unwrap_or_default()) {
        Ok(()) => "the walk stops after the task it is in; `quit` is the word that closes the window".to_owned(),
        Err(e) => format!("cannot ask the walk to stop: {e}"),
    }
}

/// The order a walk would take the cards in, as `rc run --all` picks them one after another,
/// each pick counted as landed (accepted) for the next: the projected order of the work, for
/// the window's walk view. With `tight` the picks are the smallest ready card first, as a walk picks
/// when the plan's window is tight ([`plan_tight`]). Nothing is written.
#[must_use]
pub fn walk_order(nodes: &[Node], gambits: &[Gambit], tight: bool) -> Vec<i64> {
    let mut nodes = nodes.to_vec();
    let mut order = Vec::new();
    while let Some(id) = batch_ready(&nodes, gambits, tight).map(|n| n.id) {
        order.push(id);
        if let Some(node) = nodes.iter_mut().find(|n| n.id == id) {
            node.state = TaskState::Accepted;
        }
    }
    order
}

/// The first card a batch runs: a defined task whose waits-on are each accepted, scored as
/// `rc next` scores ([`tree::rank`]): priority, then the gambits, then tree order. Implemented
/// does not count: the walk reviews and lands its own cards, so a card still implemented is
/// either before its review or one whose landing failed, and in neither case is its code on the
/// project's branch for a dependent to build on (runelite #140 started on #80's implemented,
/// and #80 was sent back seconds later, 2026-11-12). A card whose waits-on failed, left the roadmap or are
/// still open is not picked, so every pick moves a card out of defined and the loop ends; a
/// card still in progress is picked too, since a walk holds the project and such a card is one
/// a run left to continue, as the project's turn cap does. With `tight` the pick is the ready
/// card of the lowest complexity, the best scored among equals: what is left of the window
/// finishes small cards whole rather than starting a large one it cannot end.
fn batch_ready<'a>(nodes: &'a [Node], gambits: &'a [Gambit], tight: bool) -> Option<&'a Node> {
    ready_cards(nodes, gambits, tight).into_iter().next()
}

/// Every card a batch may run, in the order [`batch_ready`] would pick them: by score, or by
/// complexity with `tight`, the smallest first. A card whose run ended `implemented` is never
/// among them: it waits on its review ([`say_review_waits`]), and only a `needs revision`
/// verdict or a `failed` outcome puts it back.
#[must_use]
pub fn ready_cards<'a>(nodes: &'a [Node], gambits: &'a [Gambit], tight: bool) -> Vec<&'a Node> {
    let met = |dep: &i64| nodes.iter().any(|d| d.id == *dep && d.state == TaskState::Accepted);
    // a card sent back for revision is ready again at once: its verdict is in its notes and the
    // rerun resumes its session; only what it waits on holds it (the person, 2026-11-12: "all
    // needs revision cards should go right back into queue unless depends on changes"); a
    // failed run is parked for the rest of its walk and tried again by the next
    let candidates = nodes.iter().filter(|n| {
        matches!(n.state, TaskState::Defined | TaskState::InProgress | TaskState::NeedsRevision | TaskState::Failed)
            && matches!(n.kind, NodeKind::Task | NodeKind::Subtask)
            && n.depends_on.iter().all(met)
    });
    let mut ready: Vec<&Node> = tree::rank(nodes, candidates, gambits).into_iter().map(|s| s.node).collect();
    if tight {
        ready.sort_by_key(|n| n.complexity);
    }
    ready
}

/// How long a tool a card needs may take to answer `--version`: one that started is on the
/// path, whatever it makes of the flag, so the wait only bounds a tool that never exits.
const NEED_PROBE: std::time::Duration = std::time::Duration::from_secs(10);

/// What holds card `node` from starting in the project at `root`, named, or `None` when it can
/// start: a card it waits on whose landing is not in the project's HEAD, the base its worktree
/// starts from and every catch-up merges in, or a need of its own that is missing on the
/// machine. Runelite #80 ran 38 times on a base without its dependency's merge and without
/// `third-party/jdk8u`, and asm #53 needed `upx` (2026-11-12): a card the walk cannot finish
/// waits with the missing one named, rather than spend runs.
pub async fn prerequisites_missing(root: &Path, node: &Node) -> Option<String> {
    for dep in &node.depends_on {
        if let Some(why) = dependency_outside_base(root, *dep).await {
            return Some(why);
        }
    }
    for need in &node.needs {
        if let Some(why) = missing_need(root, need).await {
            return Some(why);
        }
    }
    None
}

/// Why the work of card `dep` is not in the project's HEAD, or `None` when it is. The card's
/// branch, while it exists, must be an ancestor of HEAD; once a landing removed the branch, a
/// commit naming the card (`Card: #<dep>`, the trailer every landing and checkpoint carries)
/// must be in HEAD when one is anywhere in the repository. A card with neither, landed by hand
/// or before the trailer, or a project without git, holds nothing.
async fn dependency_outside_base(root: &Path, dep: i64) -> Option<String> {
    let branch = worktree_branch(dep);
    if git(root, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")]).await.is_ok() {
        let merged = git(root, &["merge-base", "--is-ancestor", &branch, "HEAD"]).await.is_ok();
        return (!merged).then(|| format!("waits on #{dep}, whose branch {branch} is not merged into HEAD"));
    }
    let trailer = format!("^Card: #{dep}$");
    let anywhere = git(root, &["log", "--all", "-n", "1", "--format=%H", "--grep", &trailer]).await.ok()?;
    if anywhere.is_empty() {
        return None;
    }
    let here = git(root, &["log", "HEAD", "-n", "1", "--format=%H", "--grep", &trailer]).await.ok()?;
    here.is_empty().then(|| format!("waits on #{dep}, whose landing is not in HEAD"))
}

/// Why `need` is missing in the project at `root`, or `None` when it is there: a path that
/// exists under the project (or an absolute one), else a bare word that runs as a tool.
async fn missing_need(root: &Path, need: &str) -> Option<String> {
    if root.join(need).exists() {
        return None;
    }
    if need.contains(['/', '\\']) {
        return Some(format!("needs `{need}`, which is not in the project"));
    }
    (!tool_runs(root, need).await).then(|| format!("needs the tool `{need}`, which does not run here"))
}

/// Whether `tool` starts in `dir`: it is run with `--version`, its output dropped.
async fn tool_runs(dir: &Path, tool: &str) -> bool {
    let mut command = tokio::process::Command::new(tool);
    command
        .arg("--version")
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    match command.spawn() {
        Ok(mut child) => {
            let _ = tokio::time::timeout(NEED_PROBE, child.wait()).await;
            true
        }
        Err(_) => false,
    }
}

/// Tells once a walk, and again when the reason changes, that card `node` waits on `why`: a
/// progress line the window shows and an event on the feed. The line, when it is news.
fn say_held<S: std::hash::BuildHasher>(
    root: &Path,
    node: &Node,
    why: &str,
    said: &mut std::collections::HashMap<i64, String, S>,
) -> Option<String> {
    if said.get(&node.id).is_some_and(|w| w == why) {
        return None;
    }
    said.insert(node.id, why.to_owned());
    let text = format!("waits #{} {} · {why}", node.id, node.title);
    tracing::info!(target: crate::trace::PROGRESS, "{text}");
    crate::pair::event(root, &text);
    Some(text)
}

/// The reason [`say_review_waits`] gives a card whose run ended `implemented`.
pub const WAITS_ON_REVIEW: &str = "waits on review";

/// Tells once a walk that each card whose run ended `implemented` and has no verdict yet waits
/// on its review, the lines that were news. The queue never takes such a card
/// ([`ready_cards`] and [`tree::next_ready`] pass it over); only its verdict, `needs revision`,
/// or a `failed` outcome the person sets back to defined, puts it on the queue again. A card in
/// `skip` is one whose review is in flight and is not told.
pub fn say_review_waits<S: std::hash::BuildHasher, T: std::hash::BuildHasher>(
    root: &Path,
    nodes: &[Node],
    skip: &std::collections::HashSet<i64, S>,
    said: &mut std::collections::HashMap<i64, String, T>,
) -> Vec<String> {
    nodes
        .iter()
        .filter(|n| n.state == TaskState::Implemented && matches!(n.kind, NodeKind::Task | NodeKind::Subtask))
        .filter(|n| !skip.contains(&n.id))
        .filter_map(|n| say_held(root, n, WAITS_ON_REVIEW, said))
        .collect()
}

/// Why the acceptance command `command` cannot run in the project at `root`, or `None` when it
/// can: its program is a path that is not in the project, or a word not on the path; and for
/// cargo, a `-p` that names no package cargo knows. A card whose check cannot run fails every
/// run for a reason no worker can fix, so a card is checked as it is written and `rc check`
/// reports it.
pub async fn unrunnable(root: &Path, command: &str) -> Option<String> {
    let mut words = command.split_whitespace();
    let program = words.next()?;
    if program.contains(['/', '\\']) {
        return (!root.join(program).exists()).then(|| format!("`{command}`: `{program}` is not in the project"));
    }
    if !on_path(program) {
        return Some(format!("`{command}`: `{program}` is not on the path"));
    }
    if program == "cargo" {
        let args: Vec<&str> = words.collect();
        let package = args.windows(2).find(|w| matches!(w[0], "-p" | "--package")).map(|w| w[1]);
        if let Some(package) = package
            && probe("cargo", &["pkgid", package], root).await.is_ok_and(|o| !o.status.success())
        {
            return Some(format!("`{command}`: cargo knows no package `{package}`"));
        }
    }
    None
}

/// Whether `program` is a file in a directory of the path, as the shell would find it: on
/// Windows with one of the `PATHEXT` extensions too.
fn on_path(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else { return false };
    let extensions: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned()).split(';').map(str::to_owned).collect()
    } else {
        Vec::new()
    };
    std::env::split_paths(&path).any(|dir| {
        let file = dir.join(program);
        file.is_file()
            || extensions.iter().filter(|e| !e.is_empty()).any(|e| {
                let mut name = file.clone().into_os_string();
                name.push(e);
                Path::new(&name).is_file()
            })
    })
}

/// The acceptance commands of the open cards that cannot run in the project at `root`, one line
/// each, for `rc check`.
pub async fn unrunnable_checks(root: &Path, nodes: &[Node]) -> Vec<String> {
    let mut faults = Vec::new();
    for node in nodes.iter().filter(|n| matches!(n.kind, NodeKind::Task | NodeKind::Subtask) && n.state != TaskState::Accepted) {
        let files = ra_roadmap::tree::files_of(nodes, node).0;
        for command in named_checks(node.acceptance.as_deref()) {
            // a package or test the card itself creates is not there yet, as the run's own
            // preflight knows (asm #180); agent #256, #291 and #488 were held on it
            if names_created(&command, files) {
                continue;
            }
            if let Some(why) = unrunnable(root, &command).await {
                faults.push(format!("#{} {}: its check cannot run: {why}", node.id, node.title));
            }
        }
    }
    faults
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
    let (path, branch) = worktree_make(root, id).await?;
    seed_target(&path).await;
    Ok((path, branch))
}

/// [`worktree_add`] without the seed of its target: the parallel walk seeds inside the card's
/// own run, since a seed copies gigabytes (asm's main target is 47 GB) and, awaited in the
/// walk's loop, stopped every other card from starting or being polled while it ran (the
/// dashboard's bubbles all dropped out, 2026-11-15).
///
/// # Errors
/// When the worktree cannot be made.
pub async fn worktree_make(root: &Path, id: i64) -> anyhow::Result<(PathBuf, String)> {
    let path = root.join(WORKTREES_DIR).join(id.to_string());
    let branch = worktree_branch(id);
    if path.join(".git").exists() {
        return Ok((path, branch));
    }
    std::fs::create_dir_all(root.join(WORKTREES_DIR)).with_context(|| format!("cannot create {WORKTREES_DIR}"))?;
    // the lease comes before the files: their fresh mtimes would make cargo rebuild everything
    // in a target leased after them
    if let Err(e) = ra_providers::claude_code::lease_target(&path) {
        tracing::warn!("#{id} leased no target: {e}");
    }
    // the branch outlives a dropped worktree: a new worktree takes it up where it was
    let exists = git(root, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")]).await.is_ok();
    let target = path.to_string_lossy().into_owned();
    if exists {
        git(root, &["worktree", "add", &target, &branch]).await?;
    } else {
        git(root, &["worktree", "add", "-b", &branch, &target, "HEAD"]).await?;
    }
    // the untracked files the project's build needs, which a checkout does not bring
    // (`[worker] carry`): a file copied into the new worktree as it is in the project, a
    // directory linked, so a source tree of gigabytes is there at once and once on the disk
    if let Ok(router) = crate::router::Router::load(root) {
        carry(root, &path, &router.worker.carry, id);
    }
    // a project whose builds are refused gets the project's built binaries at the worktree's target
    if crate::router::Router::load(root).is_ok_and(|r| r.worker.builds() == 0) {
        let _ = ra_providers::claude_code::worktree_prebuilt_note(&path);
    }
    Ok((path, branch))
}

/// The tree a run of card `id` in the project at `root` acts in: the card's worktree when it has
/// one, else the project's own.
#[must_use]
pub fn card_tree(root: &Path, id: i64) -> PathBuf {
    let worktree = root.join(WORKTREES_DIR).join(id.to_string());
    if worktree.join(".git").exists() { worktree } else { root.to_path_buf() }
}

/// Gives the worktree at `tree` its own cargo target, seeded from the project's when it has
/// none yet, off the async threads. A seed that fails leaves the first build cold, never wrong.
async fn seed_target(tree: &Path) {
    let owned = tree.to_path_buf();
    match tokio::task::spawn_blocking(move || ra_providers::claude_code::seed_worktree_target(&owned)).await {
        Ok(Ok(0)) => {}
        Ok(Ok(files)) => tracing::info!("the target of {} seeded from the project's: {files} files", tree.display()),
        Ok(Err(e)) => tracing::warn!("the target of {} was not seeded: {e}", tree.display()),
        Err(e) => tracing::warn!("the target of {} was not seeded: {e}", tree.display()),
    }
}

/// Where the walk runs a card's failed tests on the project's base commit: one detached
/// worktree under the cards', moved to each base it is asked about, building in a target of
/// its own beside theirs.
const BASE_WORKTREE: &str = "base";

/// The scratch worktree of the base runs, checked out detached at `sha`, the carried files in.
async fn base_worktree(root: &Path, sha: &str) -> anyhow::Result<PathBuf> {
    let path = root.join(WORKTREES_DIR).join(BASE_WORKTREE);
    if path.join(".git").exists() {
        git(&path, &["checkout", "-q", "-f", "--detach", sha]).await?;
    } else {
        std::fs::create_dir_all(root.join(WORKTREES_DIR)).with_context(|| format!("cannot create {WORKTREES_DIR}"))?;
        // a worktree removed by hand leaves git's record of it, which a new add trips on
        let _ = git(root, &["worktree", "prune"]).await;
        git(root, &["worktree", "add", "-f", "--detach", &path.to_string_lossy(), sha]).await?;
    }
    if let Ok(router) = crate::router::Router::load(root) {
        carry_into(root, &path, &router.worker.carry, "the base worktree");
    }
    seed_target(&path).await;
    Ok(path)
}

/// Brings the `[worker] carry` entries of the project at `root` into the worktree at `path`: a
/// file copied as it is, a directory linked; one the worktree already has is left alone.
fn carry(root: &Path, path: &Path, entries: &[String], id: i64) {
    carry_into(root, path, entries, &format!("the worktree of #{id}"));
}

/// [`carry`] into the worktree `what` names in a warning.
fn carry_into(root: &Path, path: &Path, entries: &[String], what: &str) {
    for file in entries {
        let from = root.join(file);
        let to = path.join(file);
        if !from.exists() || to.exists() {
            continue;
        }
        if let Some(dir) = to.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let carried = if from.is_dir() { link_dir(&from, &to) } else { std::fs::copy(&from, &to).map(drop) };
        if let Err(e) = carried {
            tracing::warn!("cannot carry {file} into {what}: {e}");
        }
    }
}

/// Links the directory `to` to `from`: a symbolic link, or on Windows, where one needs a
/// privilege the person may not have, a junction when the link is refused.
fn link_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    // absolute on both ends: an entry of the table is written with `/`, which `mklink` refuses
    let from = std::path::absolute(from)?;
    let to = std::path::absolute(to)?;
    let to = to.as_path();
    #[cfg(unix)]
    return std::os::unix::fs::symlink(&from, to);
    #[cfg(windows)]
    {
        if std::os::windows::fs::symlink_dir(&from, to).is_ok() {
            return Ok(());
        }
        let status = std::process::Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(to)
            .arg(&from)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()?;
        if status.success() { Ok(()) } else { Err(std::io::Error::other(format!("mklink /J exited with {status}"))) }
    }
    #[cfg(not(any(unix, windows)))]
    Err(std::io::Error::other("no directory links on this platform"))
}

/// The base a card's diff is read against: `HEAD` when the run acted in the project's own
/// tree; in a worktree the merge base of the worktree's branch and the project's, so the
/// checkpoints on the card's branch count as the card's change.
async fn diff_base(project: &Path, root: &Path) -> String {
    if same_tree(project, root) {
        return String::from("HEAD");
    }
    // a merge of the project's branch still waiting for its commit (a run capped before the
    // checkpoint) leaves the merge base at the old one, and what main brought would read as
    // the card's change: the incoming tip is the base then, when it is the project's (an
    // ancestor of its branch, which may have moved on since); a merge of another branch is the
    // card's own
    let Ok(main) = git(project, &["rev-parse", "--abbrev-ref", "HEAD"]).await else { return String::from("HEAD") };
    if let Ok(incoming) = git(root, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]).await
        && git(root, &["merge-base", "--is-ancestor", &incoming, &main]).await.is_ok()
    {
        return incoming;
    }
    git(root, &["merge-base", "HEAD", &main]).await.unwrap_or_else(|_| String::from("HEAD"))
}

/// Whether `root`, where a run acted, is the project's own tree rather than a card's worktree.
fn same_tree(project: &Path, root: &Path) -> bool {
    match (std::fs::canonicalize(project), std::fs::canonicalize(root)) {
        (Ok(a), Ok(b)) => a == b,
        _ => project == root,
    }
}

/// The files of a card's branch in its worktree: what differs from the merge base with the
/// project's branch, committed in a checkpoint or not, and the new files not yet tracked.
pub async fn branch_files(project: &Path, root: &Path) -> Vec<String> {
    let base = diff_base(project, root).await;
    let mut files = Vec::new();
    for args in [&["diff", "--name-only", base.as_str()][..], &["ls-files", "--others", "--exclude-standard"][..]] {
        if let Ok(out) = git(root, args).await {
            files.extend(out.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned));
        }
    }
    files.sort();
    files.dedup();
    files
}

/// A card's file list widened by the files its branch changed: the list keeps its order, a path
/// marked new that the branch wrote loses its mark, and every changed file no entry covers (a
/// listed directory covers what lies under it) joins at the end. Asm #80 left out the files
/// its work needed, and the reviewer judged the card by the ones it named.
fn file_list(files: &[String], changed: &[String]) -> Vec<String> {
    let mut out: Vec<String> = files
        .iter()
        .map(|entry| {
            let path = tree::path_of(entry);
            if tree::is_new(entry) && changed.iter().any(|file| file == path) { path.to_owned() } else { entry.clone() }
        })
        .collect();
    for file in changed {
        let covered = out.iter().any(|entry| {
            let path = tree::path_of(entry);
            path == file || (path.ends_with('/') && file.starts_with(path))
        });
        if !covered {
            out.push(file.clone());
        }
    }
    out
}

/// The card with its file list widened by [`branch_files`] before the review, written back to
/// the roadmap so the reviewer, the next run and the person read the files the work touched. A
/// card that inherits its files from a section widens a copy of its own. The card as it was when
/// nothing changed or the roadmap cannot be written.
async fn with_branch_files(store: &Store, project: &Path, root: &Path, mut node: Node) -> Node {
    // in the project's own tree a dirty file may be the person's
    if same_tree(project, root) {
        return node;
    }
    // the project's own state and paths a list cannot carry are no card file
    let changed: Vec<String> = branch_files(project, root)
        .await
        .into_iter()
        .filter(|file| !file.starts_with("var/") && file != "rhipper.md" && !file.contains(char::is_whitespace))
        .collect();
    if changed.is_empty() {
        return node;
    }
    let own = if node.files.is_empty() {
        store.all().await.map(|nodes| tree::files_of(&nodes, &node).0.to_vec()).unwrap_or_default()
    } else {
        node.files.clone()
    };
    let files = file_list(&own, &changed);
    if files != own {
        match store.set_files(node.id, &files).await {
            Ok(()) => node.files = files,
            Err(e) => tracing::warn!("cannot widen the files of #{} from its branch: {e:#}", node.id),
        }
    }
    node
}

/// What a commit of a card's work is for: the landing of an accepted card, a checkpoint
/// after a run, or the checkpoint before a catch-up merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommitKind {
    Landing,
    Checkpoint,
    BeforeCatchUp,
}

/// The message of a commit of card `node`'s work, written as a person would: the title as
/// the subject, then what the card set out to do, what the last run reported it changed and
/// verified, the reviewer's verdict and the check, and the card's number as a trailer. The
/// person's rule of 2026-11-12: "the merges and all commits to be more professional /
/// verbose, rather than cardx blah blah".
fn card_commit_message(node: &Node, handoff: Option<&ra_context::Handoff>, kind: CommitKind, files: usize) -> String {
    let subject = match kind {
        CommitKind::Landing => node.title.clone(),
        CommitKind::Checkpoint => format!("Checkpoint: {}", node.title),
        CommitKind::BeforeCatchUp => format!("Checkpoint before catching up with the project: {}", node.title),
    };
    let mut body = Vec::new();
    if let Some(objective) = node.objective.as_deref().map(str::trim).filter(|o| !o.is_empty()) {
        body.push(format!("Objective: {objective}"));
    }
    if let Some(handoff) = handoff {
        // the report's first paragraph: what the run changed and verified, in its own words
        // the worker's words, not the head the orchestrator wrote above them
        let report: String = ra_context::without_head(handoff.report.trim()).split("\n\n").next().unwrap_or_default().trim().to_owned();
        if !report.is_empty() {
            body.push(match kind {
                CommitKind::Landing => format!("The implementing run reported:\n{report}"),
                _ => format!("The last run reported:\n{report}"),
            });
        }
        let mut proof = Vec::new();
        if let Some(verdict) = handoff.verdict.as_deref() {
            proof.push(format!("Review: {verdict}."));
        }
        if !handoff.check.trim().is_empty() {
            proof.push(format!("Check: {}.", handoff.check.trim()));
        }
        if !proof.is_empty() {
            body.push(proof.join(" "));
        }
    }
    match kind {
        CommitKind::Landing => body.push(format!("Landed by the walk on acceptance; {files} file(s) in this commit.")),
        CommitKind::Checkpoint => body.push(format!(
            "Work in progress, committed after a run so the branch carries it; {files} file(s). The card is not accepted yet."
        )),
        CommitKind::BeforeCatchUp => body.push(format!(
            "Work in progress, committed before the project's branch is merged in; {files} file(s). The card is not accepted yet."
        )),
    }
    body.push(format!("Card: #{}", node.id));
    format!("{subject}\n\n{}", body.join("\n\n"))
}

/// What every worker is asked for, at the end of its report: the commit message of its change,
/// written about the code, not the card. The landing squashes the card into one commit with it.
pub const COMMIT_PROMPT: &str = "Close your report with a section headed `## Commit`: one subject line in the imperative, at most 72 characters, saying what the change does to the code (not the card, the roadmap or the run), then a blank line and at most four short lines on why and how. No card numbers, no phase names, no mention of reviews, checks or runs.";

/// The `## Commit` section of a report: the subject and the body after it, when there is one
/// with a subject.
fn commit_section(report: &str) -> Option<(String, String)> {
    let start = report.lines().position(|l| {
        let l = l.trim().trim_start_matches('#').trim().trim_matches('*').trim_end_matches(':').trim();
        l.eq_ignore_ascii_case("commit")
    })?;
    let mut lines = report.lines().skip(start + 1).take_while(|l| !l.trim_start().starts_with("## ")).map(str::trim_end);
    let subject = lines.by_ref().map(str::trim).find(|l| !l.is_empty())?;
    let subject = subject.trim_matches('`').trim_matches('*').trim().to_owned();
    let body: Vec<&str> = lines.collect();
    let body = body.join("\n").trim().trim_matches('`').trim().to_owned();
    (!subject.is_empty()).then_some((subject, body))
}

/// The message of the one commit that lands a card: the worker's own `## Commit` section when
/// its report has one, else the card's title and the first paragraph of what the run reported.
/// Written about the change; no card number, no review or check boilerplate.
fn landing_message(node: &Node, handoffs: &[ra_context::Handoff]) -> String {
    card_landing_message(&node.title, node.notes.as_deref().unwrap_or_default(), handoffs)
}

/// [`landing_message`] from the card's parts. The message is the card's own runs': the latest
/// handoff with a `## Commit` section that is not a lint pass's (a branch's last commit was a
/// lint fix's on 2026-11-15, and three landings went under it), so a review's rerun that
/// rewrites the section is the one that lands; the notes, which a fix pass overwrites with its
/// report, come only after the handoffs.
#[must_use]
pub fn card_landing_message(title: &str, notes: &str, handoffs: &[ra_context::Handoff]) -> String {
    let own = || handoffs.iter().filter(|h| !h.report.lines().any(|l| l.trim() == format!("{LINT_PASS_FRESH}.")));
    let (subject, body) = own().rev().find_map(|h| commit_section(&h.report)).or_else(|| commit_section(notes)).unwrap_or_else(|| {
        let report = own().next().or(handoffs.last()).map(|h| h.report.as_str()).unwrap_or_default();
        let first: String = report.trim().split("\n\n").next().unwrap_or_default().trim().to_owned();
        (title.to_owned(), first)
    });
    let subject: String = subject.chars().take(100).collect();
    if body.is_empty() { subject } else { format!("{subject}\n\n{body}") }
}

/// Merges the project's branch into the card's worktree before a run, when the project has
/// moved on since the branch left it. A clean merge is a line; a conflicting one is left in
/// the tree with its markers, for the worker to resolve first ([`conflicts_block`] puts the
/// files at the head of the brief), and the checkpoint after the run completes the merge.
/// Twenty-one runelite branches from one old base conflicted with main on 2026-11-12 and
/// could not land; from here a branch is never more than one run behind. With the line comes
/// the commit the branch stood on before a clean merge, for [`prove_tree`].
async fn catch_up_worktree(root: &Path, path: &Path, node: &Node) -> Option<(String, Option<String>)> {
    let id = node.id;
    let (main, outcome) = catch_up(root, path, node).await?;
    Some(match outcome {
        CatchUp::Merged(behind, base) => (format!("  #{id}: {main} merged into {}, {behind} commit(s) behind", worktree_branch(id)), base),
        CatchUp::Conflicts(files) => {
            (format!("  #{id}: {main} merged into {} with conflicts in {files}; the worker resolves them first", worktree_branch(id)), None)
        }
        CatchUp::Refused(why) => (
            format!("  #{id}: {main} could not be merged into {}: {}; the run goes on without it", worktree_branch(id), quote_git(&why)),
            None,
        ),
    })
}

/// What merging the project's branch into a card's worktree came to.
enum CatchUp {
    /// A clean merge of this many commits of the project's branch, and the commit the card's
    /// branch stood on before it, for [`prove_tree`].
    Merged(String, Option<String>),
    /// Conflicts left in this many files, in the tree with their markers.
    Conflicts(usize),
    /// A merge git refused without naming a conflict, with git's own message; it was aborted.
    Refused(String),
}

/// One merge of `main` into the tree at `path`, `behind` commits of it, the branch standing on
/// `base` before it: a refusal without an unmerged file is aborted and carries git's message.
async fn merge_in(path: &Path, main: &str, behind: &str, base: Option<String>) -> CatchUp {
    match git(path, &["merge", "--no-edit", main]).await {
        Ok(_) => CatchUp::Merged(behind.to_owned(), base),
        Err(e) => {
            let conflicts = git(path, &["diff", "--name-only", "--diff-filter=U"]).await.unwrap_or_default();
            // rerere resolved and staged every file: the merge only waits for its commit
            if conflicts.trim().is_empty()
                && git(path, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]).await.is_ok()
                && git(path, &["commit", "-q", "--no-edit"]).await.is_ok()
            {
                return CatchUp::Merged(behind.to_owned(), base);
            }
            if conflicts.trim() == "Cargo.lock" && regenerate_lock(path, main).await {
                return CatchUp::Merged(behind.to_owned(), base);
            }
            if conflicts.trim().is_empty() {
                let _ = git(path, &["merge", "--abort"]).await;
                CatchUp::Refused(format!("{e:#}"))
            } else {
                CatchUp::Conflicts(conflicts.lines().count())
            }
        }
    }
}

/// Settles a merge whose only conflict is `Cargo.lock`: main's lock is taken, cargo rewrites it
/// for the worktree's manifests, and the merge is concluded. `false` leaves the merge as it
/// was, for the worker.
async fn regenerate_lock(path: &Path, main: &str) -> bool {
    if git(path, &["checkout", main, "--", "Cargo.lock"]).await.is_err() {
        return false;
    }
    let cargo = |offline: bool| {
        let mut command = tokio::process::Command::new("cargo");
        command.args(["update", "--workspace"]).current_dir(path);
        if offline {
            command.arg("--offline");
        }
        command.output()
    };
    let regenerated = match cargo(true).await {
        Ok(out) if out.status.success() => true,
        _ => cargo(false).await.is_ok_and(|out| out.status.success()),
    };
    if !regenerated {
        // the conflict returns as it was
        let _ = git(path, &["checkout", "-m", "--", "Cargo.lock"]).await;
        return false;
    }
    git(path, &["add", "Cargo.lock"]).await.is_ok() && git(path, &["commit", "-q", "--no-edit"]).await.is_ok()
}

/// Merges `main` into the tree at `path` the way a run's catch-up does: `None` when it went in
/// clean (a `Cargo.lock`-only conflict is settled on the way), else the number of files left
/// in conflict, 0 for a merge git refused and that was aborted.
pub async fn merge_main_in(path: &Path, main: &str) -> Option<usize> {
    match merge_in(path, main, "0", None).await {
        CatchUp::Merged(..) => None,
        CatchUp::Conflicts(files) => Some(files),
        CatchUp::Refused(_) => Some(0),
    }
}

/// The words of a git refusal that a lock held the index or a file: another git or a scanner
/// had it, and it goes once that one ends.
const LOCK_WORDS: [&str; 4] = ["index.lock", "Permission denied", "being used by another process", "Unable to create"];

/// How long a refused merge waits out a lock before its one retry.
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// Settles what it can of a merge git refused in `tree` with `text` and no conflict named, the
/// incoming side being `rev`: untracked files in the way that are each identical to the
/// incoming one are removed, a lock is waited out. The line of what was done, for the one
/// retry; `None` when nothing could be settled, and the refusal is the person's.
async fn settle_refusal(tree: &Path, rev: &str, text: &str) -> Option<String> {
    if text.contains("would be overwritten") {
        let named = overwritten_files(text);
        let cleared = clear_identical(tree, rev, &named).await;
        return (!named.is_empty() && cleared.len() == named.len())
            .then(|| format!("cleared {} untracked file(s) identical to {rev}'s: {}", cleared.len(), cleared.join(", ")));
    }
    if LOCK_WORDS.iter().any(|word| text.contains(word)) {
        tokio::time::sleep(LOCK_WAIT).await;
        return Some(format!("waited out a lock: {}", quote_git(text)));
    }
    None
}

/// Removes the untracked `files` of `tree` when every one of them is identical to its copy at
/// `rev` (line endings aside), and answers the ones removed; none when one of them differs,
/// as that one is someone's own work.
async fn clear_identical(tree: &Path, rev: &str, files: &[String]) -> Vec<String> {
    let normal = |bytes: &[u8]| String::from_utf8_lossy(bytes).replace("\r\n", "\n");
    for file in files {
        let Ok(here) = std::fs::read(tree.join(file)) else { return Vec::new() };
        let incoming = tokio::process::Command::new("git").args(["show", &format!("{rev}:{file}")]).current_dir(tree).output().await;
        match incoming {
            Ok(out) if out.status.success() && normal(&out.stdout) == normal(&here) => {}
            _ => return Vec::new(),
        }
    }
    files.iter().filter(|file| std::fs::remove_file(tree.join(file)).is_ok()).cloned().collect()
}

/// Git's message on one line for the feed: its lines trimmed and joined, clipped.
fn quote_git(text: &str) -> String {
    let joined = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ");
    joined.chars().take(300).collect()
}

/// Merges the project's branch into the card's worktree at `path`: the branch's name and what
/// the merge came to, `None` when the card's branch has every commit of it already or git
/// cannot say. A tree whose conflicts are still unresolved takes no second merge: committing it
/// would put the markers on the branch.
async fn catch_up(root: &Path, path: &Path, node: &Node) -> Option<(String, CatchUp)> {
    let main = git(root, &["rev-parse", "--abbrev-ref", "HEAD"]).await.ok()?;
    enable_rerere(root).await;
    let open = unresolved(path).await;
    if !open.is_empty() {
        return Some((main, CatchUp::Conflicts(open.len())));
    }
    let behind = git(path, &["rev-list", "--count", &format!("HEAD..{main}")]).await.ok()?;
    if behind.trim() == "0" {
        return None;
    }
    // a tree with changes cannot take a merge: a crash's leftovers, or the run's own work when
    // the catch-up comes before the review, go on the branch first
    if let Ok(dirty) = git(path, &["status", "--porcelain"]).await
        && !dirty.trim().is_empty()
    {
        let message = card_commit_message(node, None, CommitKind::BeforeCatchUp, dirty.lines().count());
        let _ = git(path, &["add", "-A"]).await;
        let _ = git(path, &["commit", "-q", "-m", &message]).await;
    }
    let base = git(path, &["rev-parse", "HEAD"]).await.ok();
    let mut outcome = merge_in(path, &main, behind.trim(), base.clone()).await;
    // a refusal is not a conflict and never the worker's: what can be settled is, and the
    // merge is tried once more (#211, #213 and #216 went back to workers with nothing to resolve)
    if let CatchUp::Refused(why) = &outcome
        && let Some(settled) = settle_refusal(path, &main, why).await
    {
        tracing::info!(target: crate::trace::PROGRESS, "#{}: {settled}; merging {main} again", node.id);
        outcome = merge_in(path, &main, behind.trim(), base).await;
    }
    Some((main, outcome))
}

/// Turns git's rerere on in the project's config, which its worktrees share: a conflict the
/// conflict pass resolved once is resolved by git when it meets the same hunks again (#211 and
/// #216 each met one three times, and a model resolved it afresh each time).
async fn enable_rerere(root: &Path) {
    for key in ["rerere.enabled", "rerere.autoupdate"] {
        let _ = git(root, &["config", key, "true"]).await;
    }
}

/// The unmerged files of a tree that still hold a conflict marker: a merge the worker has not
/// resolved yet. A file the worker resolved and left unstaged is not among them.
async fn unresolved(path: &Path) -> Vec<String> {
    let conflicts = git(path, &["diff", "--name-only", "--diff-filter=U"]).await.unwrap_or_default();
    conflicts
        .lines()
        .map(str::trim)
        .filter(|file| !file.is_empty())
        .filter(|file| std::fs::read_to_string(path.join(file)).is_ok_and(|text| text.lines().any(|l| l.starts_with("<<<<<<< "))))
        .map(str::to_owned)
        .collect()
}

/// The project's branch brought into the card's worktree after a run and before the reviewer
/// reads it, so what is reviewed is what lands (#182 passed review and failed the merged tree;
/// asm #73 and #78 passed review and conflicted at landing). `Ok` with the line of the catch-up
/// when the tree is the project's already, merged clean and passing the card's checks, or git
/// refused the merge; `Err` with the lines of why the card went back to its worker unreviewed: a
/// conflict, which the next run's brief opens on ([`conflicts_block`]), or a failed check on
/// the merged tree, at the head of the notes as a check failure of the card. A run in the
/// project's own tree has nothing to catch up.
async fn catch_up_for_review(store: &Store, id: i64, worker: &Worker) -> Result<Option<String>, Vec<String>> {
    let root = workspace_of(store);
    let Some(path) = worker.workspace.clone().filter(|p| p.join(".git").exists() && !same_tree(&root, p)) else {
        return Ok(None);
    };
    let Ok(node) = store.read(id).await else { return Ok(None) };
    let branch = worktree_branch(id);
    let Some((main, outcome)) = catch_up(&root, &path, &node).await else { return Ok(None) };
    let caught = match outcome {
        CatchUp::Merged(behind, _) => format!("  #{id}: caught up: {behind} commit(s) merged from {main} into {branch}"),
        // git refused without naming a file, and the one retry did not take: the person's, with
        // git's own words, never a worker's rerun with nothing to resolve
        CatchUp::Refused(why) => {
            let said = quote_git(&why);
            let notes = node.notes.as_deref().map(str::trim).filter(|n| !n.is_empty());
            let lead = format!("merge refused: {main} into {branch}: {said}");
            let lead = notes.map_or_else(|| lead.clone(), |notes| format!("{lead}\n\n{notes}"));
            let held = match store.set_notes(id, &lead).await {
                Ok(()) => store.set_state_by(id, TaskState::Blocked, Source::Run).await,
                Err(e) => Err(e),
            };
            let text = match held {
                Ok(()) => format!("#{id} not reviewed: git refused to merge {main} into {branch}: {said}; the card waits for the person"),
                Err(e) => format!("#{id} not reviewed: git refused to merge {main} into {branch}: {said}; it could not be held ({e:#})"),
            };
            crate::pair::event(&root, &text);
            return Err(vec![format!("  {text}")]);
        }
        CatchUp::Conflicts(files) => match conflict_pass(&root, &path, &node, worker).await {
            Ok(pass) => format!("  #{id}: caught up: {main} merged into {branch} · {pass}"),
            Err(why) => {
                let text = match store.set_state_by(id, TaskState::NeedsRevision, Source::Run).await {
                    Ok(()) => format!(
                        "#{id} not reviewed: {main} merged into {branch} with conflicts in {files} file(s); sent back to its worker to resolve them (the conflict pass: {why})"
                    ),
                    Err(e) => format!(
                        "#{id} not reviewed: {main} merged into {branch} with conflicts in {files} file(s); it could not be sent back ({e:#})"
                    ),
                };
                crate::pair::event(&root, &text);
                return Err(vec![format!("  {text}")]);
            }
        },
    };
    tracing::info!(target: crate::trace::PROGRESS, "{}", caught.trim_start());
    let Checks { commands, timeout, .. } = checks_of(&root, &node);
    if commands.is_empty() {
        return Ok(Some(caught));
    }
    // the merged tree's check is a build like the run's: a slot, the worktree's own target and
    // the lock on it, a wait that runs out failing the check
    let own_target = ra_providers::claude_code::shared_target(&path);
    let toolbox = worker.toolbox(&path).map(|toolbox| {
        toolbox.with_timeout(timeout).with_env(own_target.iter().map(|t| ("CARGO_TARGET_DIR".to_owned(), t.display().to_string())))
    });
    let failed = {
        let _slot = hold_build(&root, &path).await;
        let target = own_target.unwrap_or_else(|| crate::hub::cargo_target(&root));
        let cargo = commands.iter().find(|c| c.trim_start().starts_with("cargo")).cloned();
        let waited = match &cargo {
            Some(_) => guard_target(&root, &path, &target, timeout).await,
            None => Ok(None),
        };
        match waited {
            Ok(_target) => {
                run_checks(&toolbox, &commands, &[]).await.into_iter().find(|run| !run.passed).map(|run| (run.command, run.text))
            }
            Err(text) => Some((cargo.unwrap_or_default(), text)),
        }
    };
    let Some((command, output)) = failed else {
        return Ok(Some(format!("{caught} · check passed on the merged tree: {}", commands.join(" · "))));
    };
    // a failure outside the card's own files and crates is the project's: the review goes on
    let changed = git(&path, &["diff", "--name-only", &format!("{main}...HEAD")]).await.unwrap_or_default();
    let changed: Vec<String> = changed.lines().filter(|l| !l.trim().is_empty()).map(str::to_owned).collect();
    if let Some(outside) = outside_failure(&path, &output, &changed) {
        return Ok(Some(format!("{caught} · {}", outside_note(&root, id, &command, &outside))));
    }
    // a failure on the merged tree is the card's check failure: its output leads the notes,
    // where the rerun reads it, and the walk counts it against the check's retries
    let notes = node.notes.unwrap_or_default();
    let lead = format!("check failed: {command}\n{output}\n\n{notes}");
    let sent = match store.set_notes(id, lead.trim_end()).await {
        Ok(()) => store.set_state_by(id, TaskState::NeedsRevision, Source::Run).await,
        Err(e) => Err(e),
    };
    let text = match sent {
        Ok(()) => format!("#{id} not reviewed: the merged tree fails {command}; sent back to its worker"),
        Err(e) => format!("#{id} not reviewed: the merged tree fails {command}; it could not be sent back ({e:#})"),
    };
    crate::pair::event(&root, &text);
    Err(vec![caught, format!("  {text}")])
}

/// The card's checks and, after them, the whole suite of every crate the squash changed
/// (`cargo test -p <crate> -q`): a card's own check is often filtered to its own tests, and a
/// landing that proved only that let a card that broke another test of its crate land. A tree
/// without crates keeps its checks as they are.
pub(crate) fn merged_suite(root: &Path, mut commands: Vec<String>, changed: &[String]) -> Vec<String> {
    let mut dirs: Vec<String> = Vec::new();
    for file in changed {
        let dir = crate_dir(file);
        if dir.starts_with("crates/") && !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    for dir in dirs {
        let Ok(manifest) = std::fs::read_to_string(root.join(&dir).join("Cargo.toml")) else { continue };
        let package = manifest
            .lines()
            .map(str::trim)
            .skip_while(|l| *l != "[package]")
            .skip(1)
            .take_while(|l| !l.starts_with('['))
            .find_map(|l| l.strip_prefix("name").and_then(|r| r.trim_start().strip_prefix('=')))
            .map(|v| v.trim().trim_matches('"').to_owned());
        let Some(package) = package else { continue };
        let command = format!("cargo test -p {package} -q");
        if !commands.contains(&command) {
            commands.push(command);
        }
    }
    commands
}

/// The commands a landing's merged tree is checked with: the card's, the suites of the crates
/// it changed ([`merged_suite`]), then the project's `[worker] landing_always`, which run
/// whatever the card touched.
#[must_use]
pub fn landing_commands(root: &Path, commands: Vec<String>, changed: &[String]) -> Vec<String> {
    let mut commands = if commands.is_empty() { commands } else { merged_suite(root, commands, changed) };
    let always = crate::router::Router::load(root).map(|router| router.worker.landing_always).unwrap_or_default();
    for command in always {
        let command = command.trim().to_owned();
        if !command.is_empty() && !commands.contains(&command) {
            commands.push(command);
        }
    }
    commands
}

/// The tests a failed check of the merged tree names when every one of them panics in a crate
/// the card neither changed nor is a direct dependent of, as `name (path)` joined by commas.
/// `None` leaves the failure the card's: a compile error (a card that breaks an API breaks its
/// dependents' build), output without a panic (a timeout), or a panic in a crate the card
/// changed or one whose `Cargo.toml` names a changed crate. A dependent further down the graph
/// reads as outside.
pub(crate) fn outside_failure(root: &Path, output: &str, changed: &[String]) -> Option<String> {
    if output.contains("error[") || output.contains("could not compile") {
        return None;
    }
    let own: Vec<String> = changed.iter().map(|f| crate_dir(f)).collect();
    let names: Vec<&str> = own.iter().filter_map(|dir| dir.strip_prefix("crates/")).collect();
    let mut seen: Vec<String> = Vec::new();
    for line in output.lines() {
        let Some((_, rest)) = line.split_once("panicked at ") else { continue };
        let mut path = rest.split_whitespace().next().unwrap_or_default();
        while let Some((head, tail)) = path.rsplit_once(':')
            && tail.chars().all(|c| c.is_ascii_digit())
        {
            path = head;
        }
        let dir = crate_dir(path);
        if path.is_empty() || own.contains(&dir) || depends_on_any(root, &dir, &names) {
            return None;
        }
        let test = line.split_once("thread '").and_then(|(_, t)| t.split_once('\'')).map(|(name, _)| name);
        let entry = test.map_or_else(|| path.to_owned(), |name| format!("{name} ({path})"));
        if !seen.contains(&entry) {
            seen.push(entry);
        }
    }
    (!seen.is_empty()).then(|| seen.join(", "))
}

/// The directory a file belongs to for [`outside_failure`]: `crates/<name>` under the
/// workspace's convention, else the first component, else the root; an absolute path is read
/// from its `crates/`.
fn crate_dir(file: &str) -> String {
    let file = file.replace('\\', "/");
    let file = file.find("crates/").map_or(file.as_str(), |at| &file[at..]);
    let mut parts = file.split('/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some("crates"), Some(name), Some(_)) => format!("crates/{name}"),
        (Some(first), Some(_), _) => first.to_owned(),
        _ => String::new(),
    }
}

/// Whether the manifest of the crate at `dir` names one of the packages as a dependency, by a
/// line that opens on the name (`ra-roadmap = …`, `ra-roadmap.workspace = true`).
fn depends_on_any(root: &Path, dir: &str, names: &[&str]) -> bool {
    let Ok(manifest) = std::fs::read_to_string(root.join(dir).join("Cargo.toml")) else { return false };
    manifest
        .lines()
        .map(str::trim_start)
        .any(|line| names.iter().any(|name| line.strip_prefix(name).is_some_and(|rest| rest.starts_with([' ', '=', '.']))))
}

/// Records a failure of the merged tree outside the card as a project note, for the runs that
/// follow to read, and answers the line the card lands with.
fn outside_note(root: &Path, id: i64, command: &str, outside: &str) -> String {
    let text = format!(
        "#{id} landed over a failing {command} outside its files and their direct dependents (a crate further down may still be its): {outside}"
    );
    if let Err(e) = crate::retro::record_pair(root, &text) {
        tracing::debug!("the project note of #{id} was not written: {e:#}");
    }
    format!("check failed outside the card on the merged tree: {outside}; noted for the project")
}

/// Where the failed proof of a card's tree waits for the card's next run, under the project.
const PROOFS_DIR: &str = "var/proofs";

fn proof_path(root: &Path, id: i64) -> PathBuf {
    root.join(PROOFS_DIR).join(format!("{id}.md"))
}

/// The block at the head of a worker's brief when the last merge or checkpoint of its branch
/// failed its proof: the markers or the compile's failure, as [`prove_tree`] left them.
fn proof_block(root: &Path, id: i64) -> Option<String> {
    std::fs::read_to_string(proof_path(root, id)).ok().filter(|text| !text.trim().is_empty())
}

/// Whether a text holds a line git writes for a conflict: `<<<<<<<`, `|||||||` or `>>>>>>>`,
/// alone or before a space. The `=======` between the sides is left out: a markdown heading
/// underlines with it.
fn has_conflict_marker(text: &str) -> bool {
    text.lines().any(|line| {
        ["<<<<<<<", "|||||||", ">>>>>>>"]
            .iter()
            .any(|marker| line.strip_prefix(marker).is_some_and(|rest| rest.is_empty() || rest.starts_with(' ')))
    })
}

/// The files changed on the tree's branch since `base` that hold a conflict marker.
async fn marked_files(tree: &Path, base: &str) -> Vec<String> {
    let Ok(changed) = git(tree, &["diff", "--name-only", "--diff-filter=d", base, "HEAD"]).await else {
        return Vec::new();
    };
    changed
        .lines()
        .map(str::trim)
        .filter(|file| !file.is_empty())
        .filter(|file| std::fs::read_to_string(tree.join(file)).is_ok_and(|text| has_conflict_marker(&text)))
        .map(str::to_owned)
        .collect()
}

/// The compile of a check command: what builds everything the check would run, without running
/// it. `cargo test` gains `--no-run` (its test arguments mean nothing then); a gradle `test`
/// task becomes `testClasses`, without its `--tests` filter; any other check is its own compile.
fn compile_of(command: &str) -> String {
    let head = command.split_once(" -- ").map_or(command, |(head, _)| head).trim_end();
    let words: Vec<&str> = head.split_whitespace().collect();
    if words.first() == Some(&"cargo") && words.get(1) == Some(&"test") {
        return if words.contains(&"--no-run") { head.to_owned() } else { format!("{head} --no-run") };
    }
    let first: &str = words.first().copied().unwrap_or_default();
    let program = first.rsplit(['/', '\\']).next().unwrap_or(first).split('.').next().unwrap_or_default();
    if matches!(program, "gradle" | "gradlew" | "gw") {
        let mut out = Vec::new();
        let mut rest = words.iter();
        while let Some(word) = rest.next() {
            if *word == "--tests" {
                rest.next();
            } else if !word.starts_with("--tests=") {
                out.push(if *word == "test" {
                    "testClasses".to_owned()
                } else if let Some(project) = word.strip_suffix(":test") {
                    format!("{project}:testClasses")
                } else {
                    (*word).to_owned()
                });
            }
        }
        return out.join(" ");
    }
    command.to_owned()
}

/// Proves what was just committed on a card's branch in `tree` since `base` (a merge of the
/// project's branch, or a checkpoint) before a worker gets the tree: no changed file holds a
/// conflict marker, and the project's compile passes on a build slot. A failure is written to
/// [`proof_path`], which the card's next run reads first ([`proof_block`]); a pass clears it.
/// Conflict markers reached checkpoint commits, and a clean auto-merge duplicated members in
/// runelite (#45): a merge git calls clean is not yet a tree that builds.
async fn prove_tree(root: &Path, tree: &Path, node: &Node, worker: &Worker, base: &str, what: &str) -> Vec<String> {
    let id = node.id;
    let branch = worktree_branch(id);
    let marked = marked_files(tree, base).await;
    let mut compiles: Vec<String> = Vec::new();
    let failure = if marked.is_empty() {
        let Checks { commands, timeout, .. } = checks_of(root, node);
        for compile in commands.iter().map(|c| compile_of(c)) {
            if !compiles.contains(&compile) {
                compiles.push(compile);
            }
        }
        // the worktree's own target, as the merged tree's check builds in
        let own_target = ra_providers::claude_code::shared_target(tree);
        let toolbox = worker
            .toolbox(tree)
            .map(|t| t.with_timeout(timeout).with_env(own_target.iter().map(|t| ("CARGO_TARGET_DIR".to_owned(), t.display().to_string()))));
        let _slot = hold_build(root, tree).await;
        run_checks(&toolbox, &compiles, &[]).await.into_iter().find(|r| !r.passed).map(|failed| {
            (
                format!("the compile `{}` failed ({})", failed.command, failed.exit),
                format!(
                    "The compile `{}` fails on the tree ({}): fix it so the tree builds, keeping both the project's change and this card's:\n\n{}",
                    failed.command, failed.exit, failed.text
                ),
            )
        })
    } else {
        // the compile would only say what the markers do: they are fixed first
        Some((
            format!("conflict markers in {} file(s)", marked.len()),
            format!(
                "Conflict markers (`<<<<<<<`, `=======`, `>>>>>>>`) were committed in these files; remove every one, keeping both sides' change:\n{}",
                marked.iter().map(|f| format!("- {f}")).collect::<Vec<_>>().join("\n")
            ),
        ))
    };
    let path = proof_path(root, id);
    let Some((short, text)) = failure else {
        let _ = std::fs::remove_file(&path);
        return vec![format!("  proof: {what} on {branch} is clean and compiles: {}", compiles.join(" · "))];
    };
    let block = format!(
        "FIRST: {what} on this card's branch ({branch}) was proven before this run and failed.\n{text}\nFix this before anything else, then go on with the card. Do not commit: the orchestrator commits after your report.\n"
    );
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, block);
    let line = format!("  proof: {what} on {branch} failed, {short}; the next run fixes it first");
    tracing::warn!("#{id} {}", line.trim_start());
    crate::pair::event(root, &format!("#{id} {}", line.trim_start()));
    vec![line]
}

/// The block at the head of a worker's brief when its tree has unmerged files: the merge of
/// the project's branch left them, and the card goes nowhere until they are whole.
async fn conflicts_block(acting: &Path) -> Option<String> {
    let conflicts = git(acting, &["diff", "--name-only", "--diff-filter=U"]).await.ok()?;
    let files: Vec<&str> = conflicts.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if files.is_empty() {
        return None;
    }
    Some(format!(
        "FIRST: the project's branch was merged into this card's branch and left merge conflicts (the `<<<<<<<`, `=======`, `>>>>>>>` markers) in these files:\n{}\nResolve every one before anything else, keeping both the project's change and this card's; then go on with the card. Do not commit: the orchestrator commits the resolution after your report.\n",
        files.iter().map(|f| format!("- {f}")).collect::<Vec<_>>().join("\n")
    ))
}

/// The turns a conflict pass gets: enough to edit the hunks and read around them.
const CONFLICT_TURNS: u32 = 15;

/// The lines of a file shown before and after each conflicted hunk.
const CONFLICT_CONTEXT: usize = 3;

/// The longest hunk a conflict pass's prompt shows whole; past it the pass reads the file.
const CONFLICT_HUNK_LINES: usize = 200;

/// What a conflict pass is told: the hunks are the task, nothing else is.
const CONFLICT_INSTRUCTION: &str = "You resolve the merge conflicts of one card's branch and nothing else. The project's branch was merged into the card's and left the hunks below between `<<<<<<<`, `=======` and `>>>>>>>` markers. Edit each file so that it keeps both the project's change and the card's, with no marker left; read around a hunk only when it needs more context. Do not build, do not commit, change nothing outside the hunks, and report in a few lines how you resolved each file.";

/// The conflicted hunks of a file's text with their line numbers, each with
/// [`CONFLICT_CONTEXT`] lines around it; a long one is clipped at [`CONFLICT_HUNK_LINES`].
fn conflict_hunks(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut hunks = Vec::new();
    let mut at = 0;
    while at < lines.len() {
        if !lines[at].starts_with("<<<<<<< ") {
            at += 1;
            continue;
        }
        let end = (at..lines.len()).find(|&i| lines[i].starts_with(">>>>>>> ")).unwrap_or(lines.len() - 1);
        let from = at.saturating_sub(CONFLICT_CONTEXT);
        let to = (end + CONFLICT_CONTEXT + 1).min(lines.len());
        let mut hunk = format!("lines {}-{to}:\n", from + 1);
        for (n, line) in lines[from..to].iter().enumerate().take(CONFLICT_HUNK_LINES) {
            let _ = writeln!(hunk, "{:>6}  {line}", from + n + 1);
        }
        if to - from > CONFLICT_HUNK_LINES {
            let _ = writeln!(hunk, "        … {} line(s) more: read the file there", to - from - CONFLICT_HUNK_LINES);
        }
        hunks.push(hunk);
        at = end + 1;
    }
    hunks
}

/// The conflict pass on the tree at `tree`, whose merge of the project's branch left unmerged
/// files: one short session of the card's model, under [`CONFLICT_TURNS`], whose prompt holds
/// only the conflicted hunks, the commit subjects of both sides and the card's title, not the
/// card's brief. A pass that leaves no marker has the merge committed, and the line says so;
/// `Err` with why not, and the caller reruns the card as before. Twenty agent reruns of
/// 2026-11-12/13 were whole runs for a conflict a hunk's worth of reading resolves.
async fn conflict_pass(root: &Path, tree: &Path, node: &Node, worker: &Worker) -> Result<String, String> {
    let id = node.id;
    let said = |e: anyhow::Error| first_line(&format!("{e:#}"));
    let listed = git(tree, &["diff", "--name-only", "--diff-filter=U"]).await.map_err(said)?;
    let files: Vec<&str> = listed.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if files.is_empty() {
        return Err("git named no conflicted file".to_owned());
    }
    let mut hunks = String::new();
    for file in &files {
        let found = conflict_hunks(&std::fs::read_to_string(tree.join(file)).unwrap_or_default());
        // a deleted or binary side has no hunk to show; the whole run is the worker's
        if found.is_empty() {
            return Err(format!("{file} holds no conflict marker"));
        }
        let _ = write!(hunks, "\n## {file}\n{}", found.join("\n"));
    }
    let main = git(root, &["rev-parse", "--abbrev-ref", "HEAD"]).await.unwrap_or_else(|_| String::from("HEAD"));
    let branch = worktree_branch(id);
    let ours = git(tree, &["log", "--format=- %s", "-n", "10", &format!("{main}..HEAD")]).await.unwrap_or_default();
    let theirs = git(tree, &["log", "--format=- %s", "-n", "10", &format!("HEAD..{main}")]).await.unwrap_or_default();
    let prompt = format!(
        "{CONFLICT_INSTRUCTION}\n\nThe card: {}\n\nThe card's branch {branch}, its commits:\n{ours}\n\nThe project's branch {main}, the commits merged in:\n{theirs}\n\nThe conflicted hunks, with their line numbers:\n{hunks}",
        node.title
    );
    let provider = worker.provider().map_err(said)?;
    let model = worker.model.clone();
    tracing::info!(target: crate::trace::PROGRESS, "#{id} · conflict pass on {} in {} file(s)", short(&model), files.len());
    if provider.has_own_tools() {
        let answer = provider
            .complete(CompletionRequest {
                model: model.clone(),
                messages: vec![Message::user(prompt)],
                temperature: Some(0.2),
                max_tokens: Some(4096),
                tools: Vec::new(),
                workspace: Some(tree.to_path_buf()),
                budget_usd: None,
                resume: None,
                max_turns: Some(CONFLICT_TURNS),
                max_context: None,
                effort: Some(Effort::default().to_string()),
                read_only: false,
            })
            .await
            .map_err(|e| said(map_error(e)))?;
        worker.record_as(&model, Some("conflict"), Some(id), &answer.usage, answer.turns).await;
    } else {
        let caps = Caps { hard: Some(CONFLICT_TURNS), ..Caps::default() };
        tool_loop(provider.as_ref(), worker, (id, None), prompt, Some(tree), None, caps).await.map_err(said)?;
    }
    let left = unresolved(tree).await;
    if !left.is_empty() {
        return Err(format!("markers left in {}", left.join(", ")));
    }
    // the resolution is recorded before the commit, for the next time the hunks meet
    let _ = git(tree, &["rerere"]).await;
    git(tree, &["add", "-A"]).await.map_err(said)?;
    git(tree, &["commit", "-q", "--no-edit"]).await.map_err(said)?;
    let line = format!("conflict pass by {} resolved {}", short(&model), files.join(", "));
    crate::pair::event(root, &format!("#{id} {line}"));
    Ok(line)
}

/// The conflict pass for a landing that conflicted: the project's branch merged into the card's
/// worktree (made again from the branch when it is gone), and the pass on what that left. `Ok`
/// with the line of the pass or of a clean merge, after which the landing is tried once more.
pub(crate) async fn resolve_landing(store: &Store, id: i64, worker: &Worker) -> Result<String, String> {
    let root = workspace_of(store);
    let said = |e: anyhow::Error| first_line(&format!("{e:#}"));
    let node = store.read(id).await.map_err(said)?;
    let (path, branch) = worktree_add(&root, id).await.map_err(said)?;
    match catch_up(&root, &path, &node).await {
        Some((_, CatchUp::Conflicts(_))) => conflict_pass(&root, &path, &node, worker).await,
        Some((main, CatchUp::Merged(behind, _))) => Ok(format!("{behind} commit(s) of {main} merged into {branch} clean")),
        Some((main, CatchUp::Refused(why))) => Err(format!("git refused to merge {main} into {branch}: {}", quote_git(&why))),
        None => Err(format!("{branch} has every commit of the project's branch, and still conflicts")),
    }
}

/// Commits what a run left in the card's worktree on the card's branch, as a checkpoint: the
/// work is on the branch from its first run, not only in a working tree a crash or a removed
/// worktree would lose, and `git log card/<id>` reads as the card's story. The reviewer reads
/// the diff from the merge base ([`diff_base`]), so a checkpoint hides nothing from it. A
/// line when something was committed, with the commit the branch stood on before it for
/// [`prove_tree`]; nothing when the tree was clean or git refused.
async fn checkpoint_worktree(root: &Path, node: &Node) -> Option<(String, String)> {
    let id = node.id;
    let path = root.join(WORKTREES_DIR).join(id.to_string());
    // a merge the worker has not resolved stays open: committed, its markers would be the
    // branch's and the next run's brief would not name them
    if !path.join(".git").exists() || !unresolved(&path).await.is_empty() {
        return None;
    }
    let dirty = git(&path, &["status", "--porcelain"]).await.ok()?;
    if dirty.trim().is_empty() {
        return None;
    }
    let base = git(&path, &["rev-parse", "HEAD"]).await.ok()?;
    let task = ra_context::TaskContext::open(root, id);
    let message = card_commit_message(node, task.handoffs.last(), CommitKind::Checkpoint, dirty.lines().count());
    git(&path, &["add", "-A"]).await.ok()?;
    git(&path, &["commit", "-q", "-m", &message]).await.ok()?;
    Some((format!("  checkpoint: {} file(s) committed on {}", dirty.lines().count(), worktree_branch(id)), base))
}

/// [`checkpoint_worktree`], then [`prove_tree`] over what it committed, with the project's git
/// lock let go between them: the compile takes minutes and the other cards' landings wait on
/// the lock, not on this card's build.
async fn checkpoint_and_prove(root: &Path, node: &Node, worker: &Worker, git_lock: tokio::sync::MutexGuard<'_, ()>) -> Vec<String> {
    let checkpoint = checkpoint_worktree(root, node).await;
    drop(git_lock);
    let Some((line, base)) = checkpoint else { return Vec::new() };
    let tree = root.join(WORKTREES_DIR).join(node.id.to_string());
    let mut lines = vec![line];
    lines.extend(prove_tree(root, &tree, node, worker, &base, "the checkpoint").await);
    lines
}

/// How old an entry under the system temp directory is before a landing's sweep removes it:
/// a test running now keeps its own.
const TEMP_SWEEP_AGE: std::time::Duration = std::time::Duration::from_secs(3600);

/// Lands card `id`: its work goes onto the project's branch, the project's check proves the
/// merged tree, the card is accepted, and what the run left behind is cleared. In order: the
/// worktree's changes are committed on `card/<id>` (the title as the message), the branch is
/// merged into the project's HEAD (a conflict aborts the merge and names the files; the
/// person lands that one by hand), the project's check runs on the merged tree on a build
/// slot (a failure undoes the merge commit with `git reset --merge` and answers a [`LandCheck`],
/// which a walk sends back to the worker), the card is set accepted, and the cleanup follows: the worktree and the
/// branch, the card's own target directory, its research brief, and under the system temp
/// directory the entries of the names `[worker] temp` lists that are older than an hour. A
/// card without a worktree or a branch (a run in the project's own tree) is accepted as it
/// stands, its work being in the tree already. The person's rule of 2026-11-12: "checks for
/// making sure if a card was accepted, its merge was successful … and tasks cleanup when they
/// are done, including temp stuff".
///
/// # Errors
/// When the card is not there, is not implemented or accepted, a git step fails, the merge
/// conflicts, or the check fails on the merged tree.
pub async fn land(store: &Store, id: i64, worker: &Worker) -> anyhow::Result<Vec<String>> {
    // a merged check that failed only on the project's declared outputs rewrote them on the
    // card's branch: the landing goes once more, and a second failure sends the card back
    match land_once(store, id, worker, true).await {
        Err(e) => match e.downcast::<Regenerated>() {
            Ok(Regenerated { mut lines }) => {
                lines.extend(land_once(store, id, worker, false).await?);
                Ok(lines)
            }
            Err(e) => Err(e),
        },
        landed => landed,
    }
}

/// One attempt of [`land`]; `regenerate` lets a merged check that failed only in tests of the
/// project's `[worker] regenerate` rewrite their files on the card's branch and answer
/// [`Regenerated`] instead of a [`LandCheck`].
async fn land_once(store: &Store, id: i64, worker: &Worker, regenerate: bool) -> anyhow::Result<Vec<String>> {
    let node = store.read(id).await?;
    anyhow::ensure!(
        matches!(node.state, TaskState::Implemented | TaskState::Accepted),
        "#{id} is {}: a card lands once the reviewer accepted it (implemented), not before",
        node.state
    );
    anyhow::ensure!(!never_reviewed(store, id).await, "#{id} was never reviewed: `review {id}` judges it before it lands");
    let root = workspace_of(store);
    // one landing at a time in the project's checkout across processes: `rc land` from a shell
    // and a walk's own landings merge into the same tree, and the walk's lock is its own
    // process's alone (a hand merge was clobbered by a walk's landing, asm, 2026-11-12)
    let _landing = guard_landing(&root).await;
    let branch = worktree_branch(id);
    let path = root.join(WORKTREES_DIR).join(id.to_string());
    let mut lines = Vec::new();
    let task = ra_context::TaskContext::open(&root, id);
    let handoff = task.handoffs.last();
    // the worktree's changes, committed on the card's branch
    if path.join(".git").exists() {
        let dirty = git(&path, &["status", "--porcelain"]).await?;
        if !dirty.trim().is_empty() {
            git(&path, &["add", "-A"]).await?;
            git(&path, &["commit", "-q", "-m", &card_commit_message(&node, handoff, CommitKind::Landing, dirty.lines().count())]).await?;
            lines.push(format!("committed {} file(s) on {branch}", dirty.lines().count()));
        }
    }
    let has_branch = git(&root, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")]).await.is_ok();
    // the roadmap is the walk's, written in the project's tree with every state change and
    // never committed there; a card branch that carries a copy of it (a catch-up merge brings
    // one) cannot merge over those edits, and git refuses the whole landing (asm #139). The
    // branch takes the project's committed roadmap back before the merge
    if has_branch && path.join(".git").exists() {
        let roadmap = crate::setup::ROADMAP_FILE;
        let main = git(&root, &["rev-parse", "--abbrev-ref", "HEAD"]).await.unwrap_or_else(|_| String::from("HEAD"));
        if git(&root, &["diff", "--quiet", &main, &branch, "--", roadmap]).await.is_err() {
            git(&path, &["checkout", &main, "--", roadmap]).await?;
            git(&path, &["commit", "-q", "-m", &format!("Leave the roadmap to the project before landing (#{id})"), "--", roadmap]).await?;
            lines.push("the branch's copy of the roadmap gave way to the project's".to_owned());
        }
    }
    if !has_branch && !path.join(".git").exists() {
        // a run in the project's own tree: its work is in the tree, uncommitted; the files
        // its handoff names are committed by name, the other cards' dirty files left alone
        let files: Vec<String> = handoff.map(|h| h.changed.clone()).unwrap_or_default();
        let dirty = git(&root, &["status", "--porcelain", "--", "."]).await.unwrap_or_default();
        let own: Vec<&str> = files
            .iter()
            .map(String::as_str)
            .filter(|f| dirty.lines().any(|l| l.get(3..).is_some_and(|p| p.trim_matches('"') == *f)))
            .collect();
        if !own.is_empty() {
            let mut args = vec!["add", "--"];
            args.extend(own.iter().copied());
            git(&root, &args).await?;
            git(&root, &["commit", "-q", "-m", &landing_message(&node, &task.handoffs)]).await?;
            lines.push(format!(
                "committed {} file(s) of the handoff on {}",
                own.len(),
                git(&root, &["rev-parse", "--abbrev-ref", "HEAD"]).await.unwrap_or_default()
            ));
        }
    }
    if has_branch {
        land_branch(&root, &branch, &node, &task.handoffs, worker, regenerate, &mut lines).await?;
    }
    if node.state != TaskState::Accepted {
        store.set_state_by(id, TaskState::Accepted, Source::Run).await?;
    }
    lines.extend(clean_after(&root, id, has_branch).await);
    // one more landed card toward the next look-back pass
    crate::lookback::count_landing(&root);
    let line = format!("landed #{id} {} · {}", node.title, lines.join(" · "));
    crate::pair::event(&root, &line);
    Ok(lines)
}

/// Squashes the card's branch into one commit on the project's and proves the merged tree with
/// [`check_merged`]; a conflict, or a squash git refuses twice, answers an error. The outcome's
/// lines join `lines`.
async fn land_branch(
    root: &Path,
    branch: &str,
    node: &Node,
    handoffs: &[ra_context::Handoff],
    worker: &Worker,
    regenerate: bool,
    lines: &mut Vec<String>,
) -> anyhow::Result<()> {
    let merged = git(root, &["merge-base", "--is-ancestor", branch, "HEAD"]).await.is_ok();
    if merged {
        lines.push(format!("{branch} is in HEAD already"));
        return Ok(());
    }
    // one commit on the project's branch, squashed: the card's checkpoints and catch-up
    // merges stay on its branch, which goes once it lands, and the commit is written
    // about the change, not the card (the person: the history "should be concise and
    // changes focused, not roadmap focused", 2026-11-12)
    let mut aside = false;
    if let Err(e) = git(root, &["merge", "--squash", branch]).await {
        let conflicts = git(root, &["diff", "--name-only", "--diff-filter=U"]).await.unwrap_or_default();
        let _ = git(root, &["reset", "--merge"]).await;
        if !conflicts.trim().is_empty() {
            return Err(anyhow::Error::new(LandConflict { branch: branch.to_owned(), files: conflicts.replace('\n', ", ") }));
        }
        // no unmerged file: not a conflict, and the card stays accepted. Git's own line
        // is the reason (asm and runelite cards went back to their workers 15 times
        // for a dirty tree)
        let why = first_line(&format!("{e:#}"));
        let text = format!("{e:#}");
        let named = overwritten_files(&text);
        let cleared = if text.contains("would be overwritten") { clear_identical(root, branch, &named).await } else { Vec::new() };
        if !named.is_empty() && cleared.len() == named.len() {
            // the person's untracked copies are the card's own files: nothing of theirs to keep
            lines.push(format!("cleared {} untracked file(s) identical to {branch}'s: {why}", cleared.len()));
        } else if text.contains("would be overwritten") && !named.is_empty() {
            let mut args = vec!["stash", "push", "-q", "-u", "--"];
            args.extend(named.iter().map(String::as_str));
            git(root, &args).await?;
            aside = true;
            lines.push(format!("the person's {} file(s) set aside for the squash: {why}", named.len()));
        } else if LOCK_WORDS.iter().any(|word| text.contains(word)) {
            lines.push(format!("a file was locked, retrying: {why}"));
            tokio::time::sleep(LOCK_WAIT).await;
        } else {
            return Err(anyhow::anyhow!("the squash of {branch} failed without a conflict: {text}"));
        }
        if let Err(again) = git(root, &["merge", "--squash", branch]).await {
            let _ = git(root, &["reset", "--merge"]).await;
            if aside {
                let _ = git(root, &["stash", "pop", "-q"]).await;
            }
            return Err(anyhow::anyhow!("the squash of {branch} failed again without a conflict: {again:#}"));
        }
    }
    // the files the squash staged: the card's own, and the only ones this commit takes;
    // whatever else the checkout holds uncommitted stays the person's
    let staged = git(root, &["diff", "--cached", "--name-only"]).await.unwrap_or_default();
    let staged: Vec<String> = staged.lines().filter(|l| !l.trim().is_empty()).map(str::to_owned).collect();
    if staged.is_empty() {
        lines.push(format!("{branch} is in HEAD already"));
    } else {
        // a Rust tree is formatted as it lands, the card's files only, on a worker slot:
        // a build on push that checks the format stays green without a worker running
        // rustfmt, and the format is heavy work like any build
        if root.join("Cargo.toml").is_file() {
            let _slot = hold_build(root, root).await;
            let formatted = tokio::process::Command::new("cargo").args(["fmt", "--all"]).current_dir(root).output().await;
            if formatted.is_ok_and(|o| o.status.success()) {
                let rust: Vec<&str> = staged
                    .iter()
                    .map(String::as_str)
                    .filter(|f| Path::new(f).extension().is_some_and(|e| e.eq_ignore_ascii_case("rs")) && root.join(f).is_file())
                    .collect();
                if !rust.is_empty() {
                    let mut args = vec!["add", "--"];
                    args.extend(rust.iter().copied());
                    git(root, &args).await?;
                }
            }
        }
        let message = landing_message(node, handoffs);
        git(root, &["commit", "-q", "-m", &message]).await?;
        lines.push(format!(
            "squashed {branch} into one commit on {}: {}",
            git(root, &["rev-parse", "--abbrev-ref", "HEAD"]).await.unwrap_or_default(),
            first_line(&message)
        ));
    }
    if aside {
        match git(root, &["stash", "pop", "-q"]).await {
            Ok(_) => lines.push("the person's files are back".to_owned()),
            Err(e) => lines.push(format!("the person's files stay in `git stash`: {}", first_line(&format!("{e:#}")))),
        }
    }
    check_merged(root, node, worker, regenerate, &staged, lines).await
}

/// The project's check on the merged tree: two cards right on their own are not always right
/// together (#176 and #181 compiled apart and not merged). A reviewed card ran it on the
/// caught-up tree already; this is the last guard, for a project that moved on since the
/// review. A failure undoes the squash commit and answers a [`LandCheck`], or a
/// [`Regenerated`] when `regenerate` and the failure was only in the declared outputs.
async fn check_merged(
    root: &Path,
    node: &Node,
    worker: &Worker,
    regenerate: bool,
    staged: &[String],
    lines: &mut Vec<String>,
) -> anyhow::Result<()> {
    let id = node.id;
    let Checks { commands, timeout, .. } = checks_of(root, node);
    let commands = landing_commands(root, commands, staged);
    if !commands.is_empty() && !staged.is_empty() {
        let toolbox = worker.toolbox(root).map(|t| t.with_timeout(timeout));
        let _slot = hold_build(root, root).await;
        let runs = run_checks(&toolbox, &commands, &[]).await;
        if let Some(failed) = runs.iter().find(|r| !r.passed) {
            // a failure outside the card's files and crates is the project's: the card
            // lands and a note carries the failure (#207 failed another card's test)
            if let Some(outside) = outside_failure(root, &failed.text, staged) {
                lines.push(outside_note(root, id, &failed.command, &outside));
            } else {
                // the squash commit goes; what the checkout held uncommitted stays
                let _ = git(root, &["reset", "--merge", "HEAD~1"]).await;
                let tree = root.join(WORKTREES_DIR).join(id.to_string());
                if regenerate && let Some(line) = regenerate_outputs(root, &tree, id, worker, &runs, timeout).await {
                    lines.push(line);
                    return Err(anyhow::Error::new(Regenerated { lines: std::mem::take(lines) }));
                }
                return Err(anyhow::Error::new(LandCheck {
                    command: failed.command.clone(),
                    exit: failed.exit.clone(),
                    text: failed.text.clone(),
                }));
            }
        } else {
            lines.push(format!("check passed on the merged tree: {}", commands.join(" · ")));
            // nothing else builds a refused project's binaries in its main target, which
            // the next worktree is seeded from; a failed build is a line, not a failure
            if crate::router::Router::load(root).is_ok_and(|r| r.worker.builds() == 0) {
                lines.push(build_main_binaries(root).await);
            }
        }
    }
    Ok(())
}

/// A landing attempt whose merged check failed only in the project's declared outputs, now
/// rewritten and committed on the card's branch; the lines so far, the rewrite's the last.
#[derive(Debug)]
struct Regenerated {
    lines: Vec<String>,
}

impl std::fmt::Display for Regenerated {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.lines.last().map_or("regenerated", String::as_str))
    }
}

impl std::error::Error for Regenerated {}

/// Rewrites the files of the `[worker] regenerate` entries a merged check failed on, in the
/// card's worktree at `tree`, and commits them on its branch with a subject naming them: the
/// line saying so when every failed check failed only in tests an entry names, each command
/// passed and one wrote a file. `None` sends the card back as before.
async fn regenerate_outputs(
    root: &Path,
    tree: &Path,
    id: i64,
    worker: &Worker,
    runs: &[CheckRun],
    timeout: std::time::Duration,
) -> Option<String> {
    if !tree.join(".git").exists() {
        return None;
    }
    let config = crate::router::Router::load(root).map(|router| router.worker).unwrap_or_default();
    let failed: Vec<Vec<String>> = runs.iter().filter(|run| !run.passed).map(|run| run.tests.clone()).collect();
    let entries = config.regenerations(tree, &failed)?;
    let mut ran = Vec::new();
    for entry in entries {
        let (env, words) = entry.words().map_err(|e| tracing::warn!("#{id} cannot regenerate `{}`: {e:#}", entry.test)).ok()?;
        let toolbox = worker.toolbox(tree).ok()?.with_timeout(timeout).with_env(env);
        match toolbox.check_args(&words).await {
            Ok((true, _)) => ran.push(entry.command.clone()),
            Ok((false, output)) => {
                tracing::warn!("#{id} the regenerate command `{}` failed: {}", entry.command, first_error(&output));
                return None;
            }
            Err(e) => {
                tracing::warn!("#{id} the regenerate command `{}` did not run: {e:#}", entry.command);
                return None;
            }
        }
    }
    git(tree, &["add", "-A"]).await.ok()?;
    let staged = git(tree, &["diff", "--cached", "--name-only"]).await.ok()?;
    let files: Vec<String> = staged.lines().map(str::trim).filter(|p| !p.is_empty()).map(str::to_owned).collect();
    // nothing rewritten: the test fails on the card's code, not on its file
    if files.is_empty() {
        return None;
    }
    let message = format!("Regenerate {}\n\nRewritten by {}.", files.join(", "), ran.join(" and "));
    git(tree, &["commit", "-q", "-m", &message]).await.ok()?;
    let line = format!("regenerated {} on {}: {}", files.join(", "), worktree_branch(id), ran.join(" · "));
    crate::pair::event(root, &format!("#{id} {line}"));
    Some(line)
}

/// Builds the project's binaries into its main target after a landing's check passed, off the
/// async threads, under the slot the check holds. The line the landing carries, a failure too.
async fn build_main_binaries(root: &Path) -> String {
    let (tree, target) = (root.to_path_buf(), ra_providers::claude_code::project_target(root));
    let shown = target.display().to_string();
    match tokio::task::spawn_blocking(move || ra_providers::claude_code::build_binaries(&tree, &target)).await {
        Ok(Ok(())) => format!("binaries built into {shown}"),
        Ok(Err(e)) => format!("binaries not built into {shown}: {e}"),
        Err(e) => format!("binaries not built into {shown}: {e}"),
    }
}

/// The paths git's refusal lists as "would be overwritten by merge": the tab-indented lines.
fn overwritten_files(text: &str) -> Vec<String> {
    text.lines().filter_map(|l| l.strip_prefix('\t')).map(|l| l.trim().to_owned()).filter(|l| !l.is_empty()).collect()
}

/// Whether the reviewer accepted card `id` on its last handoff.
pub(crate) fn accepted_by_review(root: &Path, id: i64) -> bool {
    ra_context::TaskContext::open(root, id)
        .handoffs
        .last()
        .and_then(|last| last.verdict.as_deref())
        .is_some_and(|verdict| verdict.starts_with("accepted"))
}

/// The flight's id for a look-back pass: no card is numbered 0.
const LOOKBACK_SEAT: i64 = 0;

/// The runs in a row that each end sent back for revision without changing a file before the
/// card is blocked for a person; counted off the card's context
/// ([`ra_context::TaskContext::idle_rounds`]), so the count outlives the walk.
const RERUNS_A_WALK: u32 = 3;

/// Records a landing conflict of card `id` (`why`) in its context with the commits on main
/// since its base that touch its files, and at the [`ra_context::CONFLICTS_HELD`]th holds the
/// card: blocked, with [`ra_context::HELD_SUPERSEDED`] at the head of its notes, not rerun.
/// Whether it held it.
///
/// # Errors
/// When the card cannot be read, its context written, or its notes or state set.
pub async fn hold_on_conflict(store: &Store, id: i64, why: &str) -> anyhow::Result<bool> {
    let root = workspace_of(store);
    let node = store.read(id).await?;
    let args = ra_context::touching_log_args(&worktree_branch(id), &node.files);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    // a log git cannot give leaves the list empty, not the conflict unrecorded
    let log = git(&root, &args).await.unwrap_or_default();
    let mut task = ra_context::TaskContext::open(&root, id);
    let held = task.record_landing_conflict(ra_context::Handoff::landing_conflict(crate::clock::unix_now(), why, &log));
    task.save(&root)?;
    if held {
        let notes = node.notes.unwrap_or_default();
        store.set_notes(id, format!("{}\n\n{notes}", ra_context::HELD_SUPERSEDED).trim_end()).await?;
        store.set_state(id, TaskState::Blocked).await?;
    }
    Ok(held)
}

/// A landing whose merge into the project's branch conflicted; the merge was aborted.
#[derive(Debug)]
pub struct LandConflict {
    branch: String,
    files: String,
}

impl std::fmt::Display for LandConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the merge of {} conflicts in {}; the merge was aborted", self.branch, self.files)
    }
}

impl std::error::Error for LandConflict {}

/// A landing whose merged tree failed the project's check; the merge was undone. The output is
/// the failing check's, clipped as [`run_checks`] clips it.
#[derive(Debug)]
pub struct LandCheck {
    command: String,
    exit: String,
    text: String,
}

impl std::fmt::Display for LandCheck {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the merged tree fails the check `{}` ({}); the merge was undone:\n{}", self.command, self.exit, self.text)
    }
}

impl std::error::Error for LandCheck {}

/// [`land`] for a walk: the lines it answers, or the one line of why it did not, on the feed
/// too; never an error, since a card the person must land by hand is not the walk's end.
pub(crate) async fn land_or_say(store: &Store, id: i64, worker: &Worker) -> Vec<String> {
    // an implemented card whose run was killed between its check and its review has no verdict:
    // it is reviewed first, and lands only if the review leaves it implemented
    let mut reviewed = Vec::new();
    if never_reviewed(store, id).await {
        reviewed = review_after_run(store, id, worker).await;
        if never_reviewed(store, id).await || store.read(id).await.is_ok_and(|n| n.state != TaskState::Implemented) {
            return reviewed;
        }
    }
    let mut landed = land(store, id, worker).await;
    // the project moved on under the card: a conflict pass in the card's worktree resolves the
    // hunks, and the landing is tried once more; only a pass that could not sends the card back
    let mut passed = Vec::new();
    let mut failed_pass = None;
    if landed.as_ref().is_err_and(|e| e.downcast_ref::<LandConflict>().is_some()) {
        match resolve_landing(store, id, worker).await {
            Ok(line) => {
                passed.push(line);
                landed = land(store, id, worker).await;
            }
            Err(why) => failed_pass = Some(why),
        }
    }
    let mut lines = reviewed;
    lines.extend(passed);
    lines.extend(say_landing(store, id, landed, failed_pass).await);
    lines
}

/// Whether card `id` is implemented while its last handoff carries no verdict: the check passed
/// and the review never answered, so the card is not offered for landing.
async fn never_reviewed(store: &Store, id: i64) -> bool {
    let Ok(node) = store.read(id).await else { return false };
    node.state == TaskState::Implemented
        && ra_context::TaskContext::open(&workspace_of(store), id).handoffs.last().is_some_and(|h| h.verdict.is_none())
}

/// The lines of a landing as [`land_or_say`] answers them, `failed_pass` the reason the
/// conflict pass did not resolve a conflict when one ran.
async fn say_landing(store: &Store, id: i64, landed: anyhow::Result<Vec<String>>, failed_pass: Option<String>) -> Vec<String> {
    match landed {
        Ok(lines) => lines,
        // the conflict pass could not resolve it: the card goes back to its worker, whose rerun
        // merges the project's branch in first and resolves the conflicts the brief names,
        // rather than waiting on a person (asm #73 and #78, 2026-11-12); at the second conflict
        // it is held instead, since main has likely moved past it (#320 was rerun three times)
        Err(e) if e.downcast_ref::<LandConflict>().is_some() => {
            let pass = failed_pass.map(|why| format!(" (the conflict pass: {why})")).unwrap_or_default();
            let held = hold_on_conflict(store, id, &e.to_string()).await;
            let text = match held {
                Ok(true) => format!("#{id} not landed: {e}; {}", ra_context::HELD_SUPERSEDED),
                Err(err) => format!("#{id} not landed: {e}; it could not be held ({err:#}), land it by hand"),
                Ok(false) => match store.set_state(id, TaskState::NeedsRevision).await {
                    Ok(()) => {
                        format!("#{id} not landed: {e}; sent back to its worker to merge the project's branch in and resolve them{pass}")
                    }
                    Err(set) => format!("#{id} not landed: {e}; it could not be sent back ({set:#}), land it by hand"),
                },
            };
            tracing::warn!("{text}");
            crate::pair::event(&workspace_of(store), &text);
            vec![text]
        }
        Err(e) => {
            let text = match e.downcast_ref::<LandCheck>() {
                // a failed check on the merged tree is a failed check of the card: it goes back
                // to its worker with the output at the head of the notes, where the rerun reads
                // it, and the walk counts it against the check's retries (#182 sat implemented
                // until a person blocked it, 2026-11-12)
                Some(failed) => {
                    let notes = store.read(id).await.ok().and_then(|n| n.notes).unwrap_or_default();
                    let lead = format!("check failed: {}\n{}\n\n{notes}", failed.command, failed.text);
                    let sent = match store.set_notes(id, lead.trim_end()).await {
                        Ok(()) => store.set_state_by(id, TaskState::NeedsRevision, Source::Run).await,
                        Err(err) => Err(err),
                    };
                    match sent {
                        Ok(()) => format!("#{id} not landed: the merged tree fails {}; sent back to its worker", failed.command),
                        Err(err) => {
                            format!("#{id} not landed: the merged tree fails {}; it could not be sent back ({err:#})", failed.command)
                        }
                    }
                }
                // not a conflict and not the worker's: git's own words, for the person
                None => format!("#{id} not landed: {}", quote_git(&format!("{e:#}"))),
            };
            tracing::warn!("{text}");
            crate::pair::event(&workspace_of(store), &text);
            vec![text]
        }
    }
}

/// What becomes of a target just released to the project's pool: kept, or removed when the pool
/// holds more targets than the walk's width plus one.
fn pool_cap(root: &Path, target: &Path) -> String {
    let pooled = target.parent().and_then(|base| std::fs::read_dir(base).ok()).map_or(0, |dir| {
        dir.flatten().filter(|e| e.file_name().to_string_lossy().strip_prefix("wt-pool-").is_some_and(|n| n.parse::<u32>().is_ok())).count()
    });
    if pooled <= crate::allocation::runners(root) as usize + 1 {
        return "target released to the pool".to_owned();
    }
    match std::fs::remove_dir_all(target) {
        Ok(()) => "target released and removed: the pool is full".to_owned(),
        Err(e) => format!("target released, kept past the pool's size: {e}"),
    }
}

/// What a landed card leaves behind, removed: its worktree and branch, its target directory
/// under the project's, its research brief and its proof, and the old entries of the project's
/// temp names.
/// Each step says what it did or could not; none of them fails the landing.
async fn clean_after(root: &Path, id: i64, has_branch: bool) -> Vec<String> {
    let mut lines = Vec::new();
    let path = root.join(WORKTREES_DIR).join(id.to_string());
    if path.join(".git").exists() {
        match git(root, &["worktree", "remove", "--force", &path.to_string_lossy()]).await {
            Ok(_) => lines.push("worktree removed".to_owned()),
            Err(e) => lines.push(format!("worktree kept: {e:#}")),
        }
    }
    if has_branch {
        // -D: a landing squashes the branch, so git never counts it merged; the card's work is on
        // the project's branch by the time the cleanup runs
        match git(root, &["branch", "-D", &worktree_branch(id)]).await {
            Ok(_) => lines.push("branch removed".to_owned()),
            Err(e) => lines.push(format!("branch kept: {}", first_line(&format!("{e:#}")))),
        }
    }
    // a pooled target outlives the card, its build kept for the next one's lease
    if let Some(target) = ra_providers::claude_code::release_target(&path) {
        lines.push(pool_cap(root, &target));
    } else if let Some(target) = ra_providers::claude_code::shared_target(&path)
        && target.is_dir()
    {
        match std::fs::remove_dir_all(&target) {
            Ok(()) => lines.push("target removed".to_owned()),
            Err(e) => lines.push(format!("target kept: {e}")),
        }
    }
    let _ = std::fs::remove_file(brief_path(root, id));
    // a proof that failed and was fixed in a run that ran no proof after it would open a
    // later card of the same id, or a reopened one, on a job that is done
    let _ = std::fs::remove_file(proof_path(root, id));
    let names = crate::router::Router::load(root).map(|r| r.worker.temp).unwrap_or_default();
    if !names.is_empty() {
        let (count, bytes) = sweep_temp(&names, TEMP_SWEEP_AGE);
        if count > 0 {
            lines.push(format!("temp: {count} old entr{} removed, {:.1} GB", if count == 1 { "y" } else { "ies" }, bytes as f64 / 1e9));
        }
    }
    lines
}

/// Removes the entries older than `age` under each of `names` in the system temp directory,
/// and answers with how many and how many bytes. A name is a directory a project's tests make
/// (`%TEMP%\ra-asm-emit-tests`), listed in `[worker] temp`; nothing outside those is touched.
#[must_use]
pub fn sweep_temp(names: &[String], age: std::time::Duration) -> (usize, u64) {
    let temp = std::env::temp_dir();
    let cutoff = crate::clock::now().checked_sub(age);
    let (mut count, mut bytes) = (0usize, 0u64);
    for name in names {
        let dir = temp.join(name);
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            let old = meta.modified().ok().zip(cutoff).is_some_and(|(m, c)| m < c);
            if !old {
                continue;
            }
            let size = dir_size(&entry.path());
            let removed = if meta.is_dir() { std::fs::remove_dir_all(entry.path()) } else { std::fs::remove_file(entry.path()) };
            if removed.is_ok() {
                count += 1;
                bytes += size;
            }
        }
    }
    (count, bytes)
}

/// Removes the `wt-<id>` targets of the project whose card has no worktree: what landings left
/// behind before the cleanup asked the provider where a worktree builds. A line with the count
/// and the bytes freed; nothing when none was there.
fn sweep_orphan_targets(root: &Path) -> Option<String> {
    let target = ra_providers::claude_code::project_target(root);
    let (mut count, mut bytes) = (0usize, 0u64);
    for entry in std::fs::read_dir(&target).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        // a card's own target only: the pool's `wt-pool-<n>` and `wt-base` outlive the cards
        let Some(id) = name.strip_prefix("wt-").filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())) else { continue };
        if root.join(WORKTREES_DIR).join(id).exists() || !entry.path().is_dir() {
            continue;
        }
        let size = dir_size(&entry.path());
        if std::fs::remove_dir_all(entry.path()).is_ok() {
            count += 1;
            bytes += size;
        }
    }
    (count > 0).then(|| format!("targets: {count} of cards with no worktree removed, {:.1} GB", bytes as f64 / 1e9))
}

/// The bytes under a path, a file's own or a directory's whole.
fn dir_size(path: &Path) -> u64 {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => meta.len(),
        Ok(meta) if meta.is_dir() => std::fs::read_dir(path).map(|rd| rd.flatten().map(|e| dir_size(&e.path())).sum()).unwrap_or(0),
        _ => 0,
    }
}

/// The landing check of the doctor: the accepted cards whose branch is not in HEAD, and the
/// worktrees of cards that are accepted or gone. Nothing to say is the good answer.
async fn landing_check(root: &Path) -> Result<String, String> {
    let path = root.join(crate::setup::ROADMAP_FILE);
    let text = std::fs::read_to_string(&path).map_err(|_| format!("no {}", path.display()))?;
    let nodes = ra_roadmap::markdown::parse(&text).map_err(|e| format!("{} does not parse: {e:#}", path.display()))?;
    let branches = git(root, &["for-each-ref", "--format=%(refname:short)", "refs/heads/card/"]).await.unwrap_or_default();
    let mut unmerged = Vec::new();
    for branch in branches.lines() {
        let Some(id) = branch.strip_prefix("card/").and_then(|s| s.parse::<i64>().ok()) else { continue };
        let accepted = nodes.iter().any(|n| n.id == id && n.state == TaskState::Accepted);
        let merged = git(root, &["merge-base", "--is-ancestor", branch, "HEAD"]).await.is_ok();
        if accepted && !merged {
            unmerged.push(format!("#{id}"));
        }
    }
    let mut leftover = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root.join(WORKTREES_DIR)) {
        for entry in entries.flatten() {
            let Some(id) = entry.file_name().to_string_lossy().parse::<i64>().ok() else { continue };
            let open = nodes.iter().any(|n| n.id == id && n.state != TaskState::Accepted);
            if !open && entry.path().join(".git").exists() {
                leftover.push(format!("#{id}"));
            }
        }
    }
    let mut notes = Vec::new();
    if !unmerged.is_empty() {
        notes.push(format!("accepted but not merged: {} (`rc land <id>`)", unmerged.join(" ")));
    }
    if !leftover.is_empty() {
        notes.push(format!("worktrees of landed or gone cards: {}", leftover.join(" ")));
    }
    if notes.is_empty() { Ok("every accepted card's branch is in HEAD; no worktree left over".to_owned()) } else { Err(notes.join("; ")) }
}

/// The temp check of the doctor: what the project's `[worker] temp` names hold under the
/// system temp directory, and how much of it a landing would sweep.
fn temp_check(root: &Path) -> Result<String, String> {
    let names = crate::router::Router::load(root).map(|r| r.worker.temp).unwrap_or_default();
    if names.is_empty() {
        return Ok("no `[worker] temp` names; nothing is swept".to_owned());
    }
    let temp = std::env::temp_dir();
    let cutoff = crate::clock::now().checked_sub(TEMP_SWEEP_AGE);
    let (mut total, mut old) = (0u64, 0u64);
    for name in &names {
        let dir = temp.join(name);
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let size = dir_size(&entry.path());
            total += size;
            if entry.metadata().ok().and_then(|m| m.modified().ok()).zip(cutoff).is_some_and(|(m, c)| m < c) {
                old += size;
            }
        }
    }
    let text = format!(
        "{} under {}: {:.1} GB, {:.1} GB older than an hour",
        names.join(", "),
        temp.display(),
        total as f64 / 1e9,
        old as f64 / 1e9
    );
    if old > 1_000_000_000 { Err(format!("{text}; a landing sweeps it, or `rc land` any accepted card")) } else { Ok(text) }
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

/// The file two lists of paths share, a directory counting for the files under it.
fn shared_file(a: &[String], b: &[String]) -> Option<String> {
    let norm = |p: &str| p.trim().replace('\\', "/").trim_end_matches('/').to_owned();
    let (a, b): (Vec<String>, Vec<String>) = (a.iter().map(|p| norm(p)).collect(), b.iter().map(|p| norm(p)).collect());
    a.iter().filter(|x| !x.is_empty()).find_map(|x| {
        b.iter()
            .find(|y| !y.is_empty() && (x == *y || x.starts_with(&format!("{y}/")) || y.starts_with(&format!("{x}/"))))
            .map(|y| if x.len() >= y.len() { x.clone() } else { y.clone() })
    })
}

/// The ready cards in the order they should start, each with the running (or earlier-starting)
/// card it shares a file with: those that share none first, in their order, then the held-back
/// ones. Two cards on the same files land second with a conflict and rerun, so the dispatcher
/// takes the next card that does not overlap while it has one.
pub(crate) fn spread<'a>(ready: &'a [Node], nodes: &[Node], running: &[(i64, Vec<String>)]) -> Vec<(&'a Node, Option<(i64, String)>)> {
    let mut claims = running.to_vec();
    let mut left: Vec<&Node> = ready.iter().collect();
    let mut order = Vec::new();
    while !left.is_empty() {
        let blocker = |n: &Node| {
            let files = ra_roadmap::tree::files_of(nodes, n).0;
            claims.iter().find_map(|(id, held)| shared_file(files, held).map(|f| (*id, f)))
        };
        let at = left.iter().position(|n| blocker(n).is_none()).unwrap_or(0);
        let node = left.remove(at);
        let held_back = blocker(node);
        claims.push((node.id, ra_roadmap::tree::files_of(nodes, node).0.to_vec()));
        order.push((node, held_back));
    }
    order
}

/// The files each card in `ids` names or has changed on its branch, as the dispatcher reads them.
async fn claimed_files(root: &Path, nodes: &[Node], ids: &[i64]) -> Vec<(i64, Vec<String>)> {
    let mut claims = Vec::new();
    for &id in ids {
        let mut files: Vec<String> =
            nodes.iter().find(|n| n.id == id).map(|n| ra_roadmap::tree::files_of(nodes, n).0.to_vec()).unwrap_or_default();
        if let Ok(changed) = git(root, &["diff", "--name-only", &format!("HEAD...card/{id}")]).await {
            files.extend(changed.lines().filter(|l| !l.trim().is_empty()).map(str::to_owned));
        }
        claims.push((id, files));
    }
    claims
}

/// The run of one card in a parallel walk, polled on the walk's own task.
type InFlight<'a> = std::pin::Pin<Box<dyn Future<Output = anyhow::Result<Vec<String>>> + 'a>>;

/// Runs the ready cards `width` at a time, each in its own worktree, in waves: the ready cards
/// are read, up to `width` of them run together, and readiness is read again once the wave
/// ends, until no card is ready, a run fails, or with `stop_on_revision` a card needs revision.
/// A dependent starts only once its dependency's landing is in HEAD, the base its worktree
/// starts from, and its needs are on the machine ([`prerequisites_missing`]). A newer `rc`
/// installed meanwhile takes the walk over once the runs in flight end ([`handover_line`]).
///
/// # Errors
/// When the roadmap cannot be read between waves, or a worktree cannot be made.
pub async fn run_parallel(store: &Store, worker: &Worker, width: usize, stop_on_revision: bool) -> anyhow::Result<Vec<String>> {
    anyhow::ensure!(width > 0, "the width of a parallel batch is at least one");
    share_cpu_among_builds();
    // a walk that dies takes its workers with it
    ra_tools::cpu::own_children();
    let root = workspace_of(store);
    // a walk started by one moving to a newer install takes the lock as that one exits
    let lock = match take_walk_lock(&root) {
        Some(lock) => Some(lock),
        None => handed_walk_lock(&root).await,
    };
    let Some(_lock) = lock else {
        anyhow::bail!("a walk is already running in this project; `rc walk --stop` ends it after its current card")
    };
    note_walk_pid(&root);
    // what the install said at the start: a change since is a newer rc to move to
    let seen = worker.install.as_deref().and_then(crate::hub::installed);
    // the version this walk hands over to once its runs in flight end, and whether a move that
    // failed keeps it on its own rc for good
    let mut moving: Option<String> = None;
    let mut stays = false;
    probe_the_machine(&root).await;
    let mut lines = Vec::new();
    // the cards a killed walk left in progress go back on the queue before this one reads it
    lines.extend(requeue_orphans(store, &root).await);
    lines.extend(sweep_orphan_targets(&root));
    // the planned cards done a day ago are read back first: the day since them against the
    // day before, so the plan's claims are judged before the walk spends the next day
    lines.extend(crate::commands::plan::read_back(store, &root, worker).await);
    // the cards a review off the form held are judged again, and landed when accepted
    lines.extend(review_held(store, worker).await);
    let mut ran = 0;
    // the runs in flight, as a pool of `width`: when one ends the next ready card starts in
    // its slot at once, rather than the pool waiting for the slowest of a wave (the person
    // saw one card going on a two-job project for the length of a long card). Every slot
    // is polled on this one task, so two runs never interleave a read-modify-write of the
    // roadmap, which a spawned task per run could not promise
    // the project's git, one landing or checkpoint at a time across the runs in flight; and
    // the project's root, which the runs' futures borrow
    let git_lock_owned = tokio::sync::Mutex::new(());
    let root_owned = root.clone();
    let mut flight: Vec<(i64, InFlight<'_>, Seat)> = Vec::new();
    // the cards the reviewer accepted that a walk stopped before they landed are landed so a
    // restart never strands accepted work (runelite #21 and #35, 2026-11-12), each in flight on
    // a seat of its own beside the runs under the project's git lock: landed one by one before
    // the first run, a landing waiting for a worker slot held every runner idle (asm, 17 cards
    // ready and one bubble on the dashboard, 2026-11-15)
    for node in store.all().await? {
        if node.state == TaskState::Implemented && matches!(node.kind, NodeKind::Task | NodeKind::Subtask) {
            let id = node.id;
            let git_lock = &git_lock_owned;
            flight.push((
                id,
                Box::pin(async move {
                    // under the card's span, so its check reads on the card's own console card
                    // rather than an untagged one the window shows as "worker" (asm #39)
                    let span = tracing::info_span!(target: crate::trace::PROGRESS, crate::trace::CARD_SPAN, id = id);
                    let _git = git_lock.lock().await;
                    Ok(land_or_say(store, id, worker).instrument(span).await)
                }),
                Seat::Investigate,
            ));
        }
    }
    // the slow suites the landings never run, on main once a day, beside the runs on a seat of
    // their own: they take a worker slot like any check, not a runner
    if slow_due(&root, unix_now()) {
        flight.push((SLOW_SEAT, Box::pin(async move { Ok(slow_suites(store, worker, unix_now()).await) }), Seat::Investigate));
    }
    let mut stopped: Option<String> = None;
    // the cards parked past [`RERUNS_A_WALK`] or failed in this walk; the count of the rounds is
    // the card's context's, so a restarted window counts on
    let mut parked: std::collections::HashSet<i64> = std::collections::HashSet::new();
    // the cards held for a missing prerequisite, and the reason last told
    let mut held: std::collections::HashMap<i64, String> = std::collections::HashMap::new();
    // the held-back cards already said on the feed, so a wait is said once, not every tick
    let mut said_waiting: std::collections::HashSet<(i64, i64)> = std::collections::HashSet::new();
    // whether the planner filled the roadmap since the last run landed: it plans once between
    // runs, so a plan that leaves no card ready ends the walk rather than plan the next milestone too
    let mut planned = false;
    loop {
        // `stop` from the console or the window: nothing new starts, and the walk ends when
        // the runs it is in have ended
        // a walk on its way to a newer rc still takes a stop: it ends rather than hand over
        if (stopped.is_none() || moving.is_some()) && take_stop(&root) {
            stopped = Some("stopped as asked, after the cards it was in".to_owned());
        }
        // a newer rc installed: a parallel walk is never between cards, so it starts no new one
        // and hands over once the runs in flight end; the walks of 2026-11-15 kept the rc they
        // started with all day and ran cards a later install would have held
        if stopped.is_none()
            && !stays
            && let Some(dir) = &worker.install
            && let Some(version) = crate::hub::moved_to(crate::VERSION, seen.as_deref(), crate::hub::installed(dir).as_deref())
        {
            let text = handover_line(&version, flight.len());
            tracing::info!(target: crate::trace::PROGRESS, "{text}");
            crate::pair::event(&root, &text);
            lines.push(text);
            stopped = Some(format!("moving to {version}"));
            moving = Some(version);
        }
        // the slots: the machine's pool, shared by every project's walk (see [`Pool`]). A
        // card takes a slot of its model's kind before it starts, holds it for the run, and
        // the walk starts no more than its share allows; a walk with nothing in flight and
        // no slot to take waits for one rather than end
        let mut starved = false;
        // the cards the context cap ended three times in a row, split rather than started a
        // fourth time once nothing is in flight; and whether a gambit lets the planner fill
        // the next empty milestone when nothing is ready
        let mut due: Vec<i64> = Vec::new();
        let mut plans = false;
        if stopped.is_none() {
            // the cards that wait on a red main are ready again once main moves
            lines.extend(requeue_red_main(store, &root).await);
            // the project's CI on main, once an hour: a red one files its card before the
            // roadmap is read, so the walk takes it up next
            lines.extend(read_ci(store, unix_now()).await);
            let roadmap = store.load().await?;
            plans = crate::commands::run::plans_next(&roadmap.gambits);
            let router = crate::router::Router::load(&root).unwrap_or_default();
            // the runners as the table says now: the dashboard's `+`/`-` changes them while the
            // walk runs, and a lower number lets the runs in flight end without starting more
            let width = router.worker.parallel.and_then(|n| usize::try_from(n.max(1)).ok()).unwrap_or(width);
            let in_flight: std::collections::HashSet<i64> = flight.iter().map(|(id, _, _)| *id).collect();
            let others = other_walks(&root);
            let mut ready: Vec<Node> = Vec::new();
            lines.extend(say_review_waits(&root, &roadmap.nodes, &in_flight, &mut held));
            for node in ready_cards(&roadmap.nodes, &roadmap.gambits, plan_tight(&root))
                .into_iter()
                .filter(|n| !in_flight.contains(&n.id) && !parked.contains(&n.id))
            {
                if crate::plan::split::due(&root, node) {
                    due.push(node.id);
                    continue;
                }
                // a card whose prerequisites are missing waits, named, rather than spend runs
                match prerequisites_missing(&root, node).await {
                    None => ready.push(node.clone()),
                    Some(why) => lines.extend(say_held(&root, node, &why, &mut held)),
                }
            }
            // the builders first, and a card with a brief before one without: the
            // investigators run ahead down the queue, so a builder opens on a brief rather
            // than on the bare card. A card an investigator is in waits for its brief unless
            // no other card is ready for the builder
            let (briefed, bare): (Vec<Node>, Vec<Node>) = ready.into_iter().partition(|n| brief_path(&root, n.id).exists());
            let investigating: std::collections::HashSet<i64> =
                flight.iter().filter(|(_, _, s)| *s == Seat::Investigate).map(|(id, _, _)| *id).collect();
            let (waiting, bare): (Vec<Node>, Vec<Node>) = bare.into_iter().partition(|n| investigating.contains(&n.id));
            let mut taken = std::collections::HashSet::new();
            // a card that shares a file with one running waits for a card that does not; when
            // every ready card overlaps, the first runs anyway so no slot sits idle
            let queue: Vec<Node> = briefed.iter().chain(&bare).chain(&waiting).cloned().collect();
            let running: Vec<i64> = flight.iter().filter(|(_, _, s)| matches!(s, Seat::Build(_))).map(|(id, _, _)| *id).collect();
            let claims = claimed_files(&root, &roadmap.nodes, &running).await;
            let order = spread(&queue, &roadmap.nodes, &claims);
            let free_ready = order.iter().any(|(_, held)| held.is_none());
            let mut started_held = false;
            for (node, held) in order {
                if let Some((behind, file)) = held {
                    if free_ready || started_held {
                        if said_waiting.insert((node.id, behind)) {
                            crate::pair::event(&root, &format!("#{} waits behind #{behind}: both name {file}", node.id));
                        }
                        continue;
                    }
                    started_held = true;
                }
                // the queue was read before the cards ahead of this one were set up: a card the
                // person or a pairing session moved since is not started (asm #269, blocked by
                // a session and started sixteen minutes later), and the slot goes to the next
                if let Some(line) = skipped_now(store, node.id).await {
                    tracing::info!(target: crate::trace::PROGRESS, "{line}");
                    crate::pair::event(&root, &line);
                    lines.push(line);
                    continue;
                }
                let kind =
                    Pool::kind_of(&router.route(node, crate::router::IMPLEMENTER, &worker.model, rejections(&root, node.id)).0.model);
                let held = flight.iter().filter(|(_, _, s)| *s == Seat::Build(kind)).count();
                let Some(slot) = Pool::take(&root, kind, held, width, others, router.worker.borrow()) else {
                    starved = starved || held == 0;
                    continue;
                };
                let id = node.id;
                // a builder on a card an investigator is still in: the brief, when it lands,
                // is the next run's; the investigator goes on, its seat is its own
                taken.insert(id);
                let (path, branch) = worktree_make(&root, id).await?;
                // the project's branch comes into the card's before the run: a card that
                // started from an older main lands without a conflict later, and a conflict
                // there is now is the worker's first job (the brief names the files)
                let merged = match catch_up_worktree(&root, &path, node).await {
                    Some((line, merge_base)) => {
                        lines.push(line);
                        relocate_handoff_points(&root, &path, id);
                        merge_base
                    }
                    None => None,
                };
                let tree = path.clone();
                let proved = node.clone();
                let runner = worker.clone().with_worktree(path, branch);
                // the card's span: every progress line of the run carries the card, so six
                // runs' steps stay apart in the activity file and on the window's cards
                // on the progress target: the filter lets that target through and no other,
                // and a span the filter drops is no span at all
                let span = tracing::info_span!(target: crate::trace::PROGRESS, crate::trace::CARD_SPAN, id);
                let (git_lock, root_ref) = (&git_lock_owned, &root_owned);
                flight.push((
                    id,
                    Box::pin(
                        async move {
                            let _slot = slot;
                            // the card's target is seeded here, in its own run: the copy takes
                            // minutes on a big project and the walk goes on with the others
                            seed_target(&tree).await;
                            // a clean merge is proven before the worker gets the tree, inside
                            // the run's future so its compile stalls no other card; a failure
                            // is the run's first job
                            let mut out = match &merged {
                                Some(merge_base) => {
                                    prove_tree(root_ref, &tree, &proved, &runner, merge_base, "the merge of the project's branch").await
                                }
                                None => Vec::new(),
                            };
                            // the ledger's mark before the run: the rows after it are the runs'
                            let mark = match runner.ledger() {
                                Some(ledger) => ledger.last_id().await.ok(),
                                None => None,
                            };
                            out.extend(run(store, id, &runner).await?);
                            // what follows the run happens inside its own future, so the
                            // other runs are read while it merges and compiles: a landing
                            // awaited in the walk's loop stalled every worker behind it (the
                            // runelite walk, half an hour, 2026-11-12). The landings and the
                            // checkpoints take the project's git lock one at a time
                            let node = store.read(id).await?;
                            // the card's claimed saving is judged on its own rows, as it lands
                            if let (Some(mark), Some(ledger)) = (mark, runner.ledger()) {
                                out.extend(crate::retro::landed(root_ref, &node, ledger, mark).await);
                            }
                            let git_guard = git_lock.lock().await;
                            if node.state == TaskState::Implemented {
                                out.extend(land_or_say(store, id, &runner).await);
                                drop(git_guard);
                            } else {
                                out.extend(checkpoint_and_prove(root_ref, &node, &runner, git_guard).await);
                            }
                            Ok(out)
                        }
                        .instrument(span),
                    ),
                    Seat::Build(kind),
                ));
            }
            // the investigators: one a ready card without a brief, which no builder has, up to
            // the project's count; they read the project's own tree and build nothing, so
            // they sit in no pool
            let seats = router.worker.investigators();
            let mut reading = investigating.len();
            for node in bare.iter().filter(|n| !taken.contains(&n.id)) {
                if reading >= seats {
                    break;
                }
                reading += 1;
                let id = node.id;
                let reader = worker.clone();
                let span = tracing::info_span!(target: crate::trace::PROGRESS, crate::trace::CARD_SPAN, id);
                flight.push((id, Box::pin(async move { investigate(store, id, &reader).await }.instrument(span)), Seat::Investigate));
            }
        }
        // every ten landed cards a look-back pass reads the look-backs and the gotchas and plans
        // what they call for, beside the runs on a seat of its own (no card is numbered 0)
        if stopped.is_none() && crate::lookback::due(&root) && !flight.iter().any(|(id, _, _)| *id == LOOKBACK_SEAT) {
            let planner = worker.clone();
            flight.push((LOOKBACK_SEAT, Box::pin(async move { crate::lookback::pass(store, &planner).await }), Seat::Investigate));
        }
        if flight.is_empty() {
            // a card due a split is split here, with no run in flight to stall: the planner
            // proposes the remaining work as cards and the card is blocked with it
            if !due.is_empty() {
                for id in due {
                    match crate::plan::split::split(store, &root, id, worker).await {
                        Ok(line) => lines.push(line),
                        Err(error) => {
                            let line = format!("the split of #{id} failed and it waits for a person: {error:#}");
                            tracing::warn!("{line}");
                            crate::pair::event(&root, &line);
                            lines.push(line);
                        }
                    }
                    parked.insert(id);
                }
                continue;
            }
            if starved && stopped.is_none() {
                // every slot of the kind is another project's: wait for one
                tokio::time::sleep(WALK_TICK).await;
                continue;
            }
            // the move waited for the runs in flight: the new rc takes the walk up now, with
            // the same arguments, so the runners stay as they were. A stop or a full stop that
            // came meanwhile ends the walk instead
            if let Some(version) = moving.take()
                && stopped.as_deref() == Some(format!("moving to {version}").as_str())
                && let Some(dir) = &worker.install
            {
                match (worker.relaunch)(dir, &root) {
                    Ok(pid) => {
                        let text = format!("moved to {version} · ran {ran} card(s) · pid {pid}");
                        crate::pair::event(&root, &format!("walk {text}"));
                        write_digest(store, &root, None, None, None).await;
                        lines.push(text);
                        return Ok(lines);
                    }
                    Err(error) => {
                        // the walk stays on its own rc and goes on; it does not try again
                        let text = format!("the walk stays on {}: {error:#}", crate::VERSION);
                        crate::pair::event(&root, &text);
                        lines.push(text);
                        stays = true;
                        stopped = None;
                        continue;
                    }
                }
            }
            let mut why = "no card is ready".to_owned();
            // a `plan-next` gambit lets the planner fill the next empty milestone; its cards
            // are the next picks
            if stopped.is_none() && plans && !planned {
                planned = true;
                match crate::commands::run::plan_next(store, &root, worker).await {
                    Ok(Some(plan_lines)) => {
                        lines.extend(plan_lines);
                        continue;
                    }
                    Ok(None) => "no card is ready and no milestone is left to plan".clone_into(&mut why),
                    Err(error) => why = format!("no card is ready · planning the next milestone failed: {error:#}"),
                }
            }
            let text = format!("ran {ran} card(s) · {}", stopped.unwrap_or(why));
            // on the feed as well: a detached walk's own lines go nowhere, and walks stopped by
            // the plan's week read as dying with no word (asm and runelite, 2026-11-12)
            crate::pair::event(&root, &format!("walk {text}"));
            write_digest(store, &root, None, None, None).await;
            lines.push(text);
            return Ok(lines);
        }
        // a run ends, or a tick passes: a card re-queued or made ready, a runner or a worker slot
        // added on the dashboard, is taken then, not when the next run happens to end
        let Ok((at, outcome)) = tokio::time::timeout(WALK_TICK, poll_one(&mut flight)).await else {
            continue;
        };
        let (id, _, seat) = flight.remove(at);
        match (seat, outcome) {
            // an investigator's end is a brief or a line in the log, never the walk's end
            (Seat::Investigate, Ok(brief_lines)) => lines.extend(brief_lines),
            (Seat::Investigate, Err(error)) => {
                let text = format!("the investigation of #{id} failed: {error:#}");
                tracing::warn!("{text}");
                crate::pair::event(&root, &text);
                lines.push(text);
            }
            (Seat::Build(_), Ok(run_lines)) => {
                // the checkpoint or the landing already happened inside the run's future
                lines.extend(run_lines);
                ran += 1;
                planned = false;
                // a card cancelled while it ran is gone from the roadmap: the walk says so and
                // goes on (asm #488 and agent #674, cancelled mid-run on 2026-11-15, ended both
                // walks with "no node" and no line saying why)
                let sent_back = match store.read(id).await {
                    Ok(node) => node.state == TaskState::NeedsRevision,
                    Err(error) => {
                        let text = format!("#{id} left the roadmap while it ran ({error:#}); the walk goes on");
                        tracing::warn!("{text}");
                        crate::pair::event(&root, &text);
                        lines.push(text);
                        false
                    }
                };
                if stop_on_revision && stopped.is_none() && sent_back {
                    stopped = Some(format!("stopped: #{id} needs revision"));
                }
                // a card sent back run after run is parked until the next walk: a rerun that
                // ends in revision at once otherwise starts again at once, and asm ran #131 270
                // times and #132 234 times in an afternoon, a turn each (2026-11-12)
                // only runs that changed nothing count: work in progress is never cut, a card
                // going round without touching a file is stuck. The count is read off the card's
                // handoffs, so a window restarted between its rounds does not start it again
                // (1024 restarts within a minute of a card's last event). A card the reviewer
                // accepted is sent back by the landing, not by its work: it never counts
                // The idle rounds are asked first: their line says the card moved nothing; a card
                // that moved files and was still rejected three times is then blocked by the
                // send-back count alone
                if sent_back && !accepted_by_review(&root, id) {
                    let count = ra_context::TaskContext::open(&root, id).idle_rounds();
                    if count < RERUNS_A_WALK {
                        if rejected_by_review(&root, id)
                            && let Some(line) = crate::hub::block_after_send_backs(store, &root, id).await
                        {
                            parked.insert(id);
                            lines.push(line);
                        }
                    } else {
                        parked.insert(id);
                        // blocked, not only parked: a card sent back three times in a row is
                        // one its worker cannot move (a missing toolchain or fixture, asm #53
                        // and #80 ran nine times saying so); the next walk would run it three
                        // more times. Its last answer says why, and a person sets it defined
                        let why = ra_context::TaskContext::open(&root, id)
                            .handoffs
                            .last()
                            .map(|h| first_line(ra_context::without_head(&h.report)).chars().take(200).collect::<String>())
                            .unwrap_or_default();
                        let held = store.set_state_by(id, TaskState::Blocked, Source::Run).await.is_ok();
                        let text = if held {
                            format!(
                                "#{id} blocked: sent back {count} times in a row without changing a file; a person sets it defined again. Its last answer: {why}"
                            )
                        } else {
                            format!(
                                "#{id} parked: sent back {count} times in a row without changing a file; it waits for the next walk or a person"
                            )
                        };
                        tracing::warn!("{text}");
                        crate::pair::event(&root, &text);
                        lines.push(text);
                    }
                }
            }
            (Seat::Build(_), Err(error)) => {
                // a full stop (the plan's limit, the provider gone) stops the walk; a card's own
                // failure (the content filter on one card, runelite #56) is that card's: it waits
                // for a person and the other runners go on (the walk stopped starting cards on
                // it and runelite ran one card of four, 2026-11-12)
                // a steer past a ladder mark names the week too, and is one card's (2026-11-14)
                let text = format!("{error:#}");
                let full_stop = plan_spent(&root)
                    || ["unreachable", "logged in", "authentication"].iter().any(|word| text.to_ascii_lowercase().contains(word));
                if full_stop {
                    stopped.get_or_insert(format!("stopped: the run of #{id} failed: {text}"));
                } else {
                    parked.insert(id);
                    let line = format!("#{id} failed and waits for a person; the walk goes on: {}", first_line(&text));
                    tracing::warn!("{line}");
                    crate::pair::event(&root, &line);
                    lines.push(line);
                }
            }
        }
    }
}

/// What a run in a walk's flight sits on: a builder's slot of the machine's pool, or an
/// investigator's seat, which is the project's own and holds no build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Seat {
    Build(PoolKind),
    Investigate,
}

/// Where a card's research brief lives, under the project's `var`.
const BRIEFS_DIR: &str = "var/briefs";

/// The most turns an investigator takes: reading, not building, forty turns cover a card.
const INVESTIGATE_TURNS: u32 = 40;

/// The path of a card's research brief.
fn brief_path(root: &Path, id: i64) -> PathBuf {
    root.join(BRIEFS_DIR).join(format!("{id}.md"))
}

/// A card's research brief, when an investigator left one.
fn research_brief(root: &Path, id: i64) -> Option<String> {
    std::fs::read_to_string(brief_path(root, id)).ok().map(|t| t.trim().to_owned()).filter(|t| !t.is_empty())
}

/// What the investigator is told: read, do not build, and leave the builder a brief.
const INVESTIGATE_INSTRUCTION: &str = "You are an investigator of a roadmap-driven orchestrator: you research one card so that a builder, who starts after you, can make its change with little reading and no exploratory builds. You have the reading and search tools and nothing else: you cannot edit, run a shell or build, and you do not try. Read the code the card touches and what it will call, and answer with a brief for the builder, in Markdown, under 150 lines, with these sections: `Where` (each file and the line region that matters, as `path:line`), `What exists` (what the current code does there, the types and helpers to reuse, the tests that cover it), `The change` (the exact edits, in order, each naming its file and the shape of the code; concrete enough to write from, short enough to read), `Leave alone` (what looks related but is not the card's), `Pitfalls` (what would break, what the acceptance checks, platform quirks you found), and `Verify` (the one check that proves the change, and whether a build is needed at all before reporting). Say what you verified by reading and what you could not; do not guess at code you did not open.";

/// Investigates `id` for the walk: a read-only run of the project's implementer model in the
/// project's own tree, which leaves the card's research brief in `var/briefs/<id>.md` for
/// the builder to open on; the brief is not a handoff and the card keeps its state. The pair
/// feed hears of the brief; the ledger books the usage under the investigator role.
///
/// # Errors
/// When the card is not there, the route has no provider, or the model did not answer.
pub async fn investigate(store: &Store, id: i64, worker: &Worker) -> anyhow::Result<Vec<String>> {
    let node = store.read(id).await?;
    anyhow::ensure!(node.kind != NodeKind::Project, "#{id} is a project; investigate a task");
    let root = workspace_of(store);
    let worker = worker.for_card(&root, &node)?;
    let model = worker.model.clone();
    let provider = worker.provider()?;
    let brief = context_of(store, &node).await?;
    let project = project_context(store);
    let mut messages = vec![Message::system(INVESTIGATE_INSTRUCTION)];
    if !project.is_empty() {
        messages.push(Message::system(format!("The project, from {}:\n{project}", ra_context::PROJECT_FILE)));
    }
    messages.push(Message::user(brief.text));
    tracing::info!(target: crate::trace::PROGRESS, "#{id} {} · investigated by {}", node.title, short(&model));
    crate::pair::event(&root, &format!("investigate #{id} {} · {} · pid {}", node.title, short(&model), std::process::id()));
    let answer = provider
        .complete(CompletionRequest {
            model: model.clone(),
            messages,
            temperature: Some(0.2),
            max_tokens: Some(4096),
            tools: Vec::new(),
            workspace: Some(root.clone()),
            budget_usd: None,
            resume: None,
            max_turns: Some(INVESTIGATE_TURNS),
            max_context: None,
            effort: Some(Effort::default().to_string()),
            read_only: true,
        })
        .await
        .map_err(map_error)?;
    worker.record_as(&model, Some("investigator"), Some(id), &answer.usage, answer.turns).await;
    let path = brief_path(&root, id);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot make {}", dir.display()))?;
    }
    let text = format!("# Brief for #{id} {}\n\nBy {} on {}.\n\n{}\n", node.title, short(&model), clock(unix_now()), answer.text.trim());
    std::fs::write(&path, &text).with_context(|| format!("cannot write {}", path.display()))?;
    let line = format!(
        "brief #{id} {} by {} · turns {} · in {} out {} · {}",
        node.title,
        short(&model),
        answer.turns,
        answer.usage.prompt_tokens,
        answer.usage.completion_tokens,
        cost_text(&answer.usage)
    );
    crate::pair::event(&root, &line);
    Ok(vec![line])
}

/// Drives the runs in flight together on the one task until one of them ends, and answers
/// which and with what. The runs overlap in their waits, the model and the checks, not in
/// their bookkeeping.
async fn poll_one<T>(flight: &mut [(i64, std::pin::Pin<Box<dyn Future<Output = T> + '_>>, Seat)]) -> (usize, T) {
    std::future::poll_fn(|cx| {
        for (at, (_, future, _)) in flight.iter_mut().enumerate() {
            if let std::task::Poll::Ready(value) = future.as_mut().poll(cx) {
                return std::task::Poll::Ready((at, value));
            }
        }
        std::task::Poll::Pending
    })
    .await
}

/// The kind of slot a run takes from the machine's pool: Fable runs on Fable slots, every
/// other model on the Opus slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolKind {
    Opus,
    Fable,
}

/// The machine's pool of runs across every project's walk: six Opus slots, shared. A
/// project's share is its `[worker] parallel` of them (two by default); an Opus project
/// borrows the slots no other walking project needs and gives them back as its runs end
/// once another project walks. A Fable project runs one card at a time, its own slot, in no
/// pool: as many Fable projects as walk. The slots are the hub's locks, held by the run and
/// released with it, dead or done. Six runs write code; what made six too heavy was six
/// builds at once, which the machine's four shared build slots ([`crate::hub::BUILD_SLOTS`])
/// now keep to four across every project. The person's rule of 2026-11-12: "i want the 6x, just don't do stupid shit
/// like having them all building with gradle".
pub struct Pool;

impl Pool {
    /// The Opus slots on the machine: twelve writers across the projects, four of whom may
    /// build at once ([`crate::hub::BUILD_SLOTS`]). The person's rule of 2026-11-12: "the
    /// pool to be 12 opus researchers, and 4 runners overall".
    pub const OPUS: usize = 12;
    /// The Opus slots a walking project is owed, which another project's borrowing leaves it.
    pub const OPUS_SHARE: usize = 2;

    /// The kind of slot a model runs on.
    #[must_use]
    pub fn kind_of(model: &str) -> PoolKind {
        if model.to_ascii_lowercase().contains("fable") { PoolKind::Fable } else { PoolKind::Opus }
    }

    /// Takes a slot of `kind` for the walk of the project at `root`, which holds `held` of
    /// them, is owed `width` Opus slots as its own share, and shares the machine with `others`
    /// walking Opus projects; `None` when the share or the pool is out. A Fable walk takes its
    /// project's one slot and no other. With `borrow` off an Opus walk holds to `width`
    /// exactly.
    #[must_use]
    pub fn take(root: &Path, kind: PoolKind, held: usize, width: usize, others: usize, borrow: bool) -> Option<crate::hub::Guard> {
        match kind {
            PoolKind::Fable => {
                // the project's runners, named by the project: no pool across them
                if held >= width.max(1) {
                    return None;
                }
                let name = root.file_name().map_or_else(|| "project".to_owned(), |n| n.to_string_lossy().to_ascii_lowercase());
                (0..width.max(1)).find_map(|seat| crate::hub::try_guard(&format!("fable-{name}-{seat}")).ok().flatten())
            }
            PoolKind::Opus => {
                let size = Self::OPUS;
                let project = crate::hub::project_name(root);
                let allowed = if crate::hub::rank_of(root).is_some() {
                    // a priority order: the projects ahead of this one take what they need,
                    // the slots they hold and the cards they have ready, and this one has the
                    // rest; the first project with work has the whole pool
                    let demand: usize = higher_walking(root)
                        .iter()
                        .map(|dir| {
                            let name = crate::hub::project_name(dir);
                            let ready =
                                crate::hub::digests().into_iter().find(|d| d.name.eq_ignore_ascii_case(&name)).map_or(0, |d| d.ready);
                            let held = crate::hub::held_by("pool-opus", size, &name);
                            ahead_demand(held, ready, crate::allocation::runners(dir) as usize)
                        })
                        .sum();
                    // never past the project's own runners: the order decides who is served
                    // first, the runners how many it runs (the agent at four runners took nine
                    // slots the order left it, 2026-11-12)
                    size.saturating_sub(demand).min(width.max(1))
                } else if borrow {
                    // the share, and an equal part of what the pool holds beyond every walking
                    // project's share: one project alone has the four, two have two each
                    let walking = others + 1;
                    (width.max(1) + size.saturating_sub(Self::OPUS_SHARE * walking) / walking).min(size)
                } else {
                    width.max(1).min(size)
                };
                if held >= allowed {
                    return None;
                }
                (0..size).find_map(|slot| crate::hub::try_guard_as(&format!("pool-opus-{slot}"), &project).ok().flatten())
            }
        }
    }
}

/// The seats a project ahead in the order keeps from the ones behind it: what it holds and what
/// it has ready, but never more than its runners can use (or than it already holds). Uncapped,
/// the agent's dozens of ready cards at six runners left asm no seat with the pool half free
/// (2026-11-13).
pub(crate) fn ahead_demand(held: usize, ready: usize, runners: usize) -> usize {
    (held + ready).min(runners.max(held))
}

/// The projects ahead of `root` in the hub's priority order that are walking now on Opus
/// slots: their demand comes first. Empty without an order, or for the first project.
pub(crate) fn higher_walking(root: &Path) -> Vec<PathBuf> {
    let Some(mine) = crate::hub::rank_of(root) else { return Vec::new() };
    let here = std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf());
    crate::hub::recent()
        .into_iter()
        .filter(|dir| std::path::absolute(dir).map_or(true, |d| d != here))
        .filter(|dir| crate::hub::rank_of(dir).is_some_and(|rank| rank < mine))
        .filter(|dir| walk_running(dir))
        .filter(|dir| project_kind(dir) == PoolKind::Opus)
        .collect()
}

/// How many other projects the window knows are walking now on Opus slots: the pool's Opus
/// shares are theirs. A Fable project's walk takes nothing from the Opus pool and counts for
/// nothing here.
pub(crate) fn other_walks(root: &Path) -> usize {
    let here = std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf());
    crate::hub::recent()
        .into_iter()
        .filter(|dir| std::path::absolute(dir).map_or(true, |d| d != here))
        .filter(|dir| walk_running(dir))
        .filter(|dir| project_kind(dir) == PoolKind::Opus)
        .count()
}

/// The pool kind a project's cards run on by default: the model of its `[[route]]` rule that
/// fits every card as implementer, else the agent's default model.
fn project_kind(root: &Path) -> PoolKind {
    let router = crate::router::Router::load(root).unwrap_or_default();
    let model = router
        .rules
        .iter()
        .find(|rule| rule.complexity.is_none() && rule.kind.is_none() && rule.role.eq_ignore_ascii_case(crate::router::IMPLEMENTER))
        .map_or(crate::ui::DEFAULT_MODEL, |rule| rule.model.as_str());
    Pool::kind_of(model)
}

/// The work of [`run`], with the usage of the session, its cost reported or computed, and the
/// count of the gotchas it hit beside the log lines.
#[allow(clippy::too_many_lines)]
async fn run_task(store: &Store, node: Node, worker: &Worker, roles: &Roles) -> anyhow::Result<(Vec<String>, Usage, usize)> {
    let id = node.id;
    // the state as it is now, not as the caller read it: an absolute pass or a fix pass starts
    // after a run whose card the person may have blocked meanwhile
    if let Some(state) = state_now(store, id).await
        && held_by_person(state)
    {
        let line = not_started_line(id, &node.title, state);
        tracing::info!(target: crate::trace::PROGRESS, "{line}");
        return Ok((vec![line], Usage::default(), 0));
    }
    let provider = worker.provider()?;
    store.set_state_by(id, TaskState::InProgress, Source::Run).await?;

    let Prepared { workspace, root, mut task, project, budget, caps, effort } = prepare(store, &node, worker, true).await?;
    // a fix pass reads its points first, the verdict's or the lint's: they are the task, the
    // card is the setting
    let lint_pass = is_lint_pass(worker);
    if let Some(fix) = &worker.fix {
        task.brief = format!("{fix}\n\n{}", task.brief);
    }
    let workspace_dir = workspace.clone();
    // where the changes land: the worktree when the run has one, else the project
    let acting = workspace_dir.clone().unwrap_or_else(|| root.clone());
    // a lint-only pass needs the warnings and the lines around them, not the card's context
    // (asm #237: 14 turns at 199k a turn on the resumed session for five warnings)
    if lint_pass {
        task.brief = lint_prompt(&acting, worker.fix.as_deref().unwrap_or_default());
        task.outlines.clear();
        task.review = None;
    }
    // a merge of the project's branch that left conflicts in the tree is the run's first job
    if let Some(block) = conflicts_block(&acting).await {
        task.brief = format!("{block}\n{}", task.brief);
    }
    // and so is a merge or a checkpoint of the branch that failed its proof
    if let Some(block) = proof_block(&root, id) {
        task.brief = format!("{block}\n{}", task.brief);
    }
    // what happened to the tree since the last run left it, a merge, a reset or commits on top:
    // the rerun hears it first, and the head of this run's handoff keeps it (asm #120 was rerun
    // on a tree reset and merged under it without a word)
    let base = diff_base(&root, &acting).await;
    let since = match task.handoffs.last().and_then(ra_context::tip_of) {
        Some(tip) => crate::review::since_last(&acting, tip).await,
        None => None,
    };
    if let Some(since) = &since
        && !lint_pass
    {
        task.brief = format!("The tree changed since the last run of this card: {since}\n\n{}", task.brief);
    }
    // what is dirty before the run, so the handoff names only what the run changed
    let dirty_before = git_status(&acting).await;
    // set back by [`MTIME_SLACK`]: a file's time comes off a coarser clock than `now`, and a
    // write in the run's first tick read as older than the run
    let run_started = crate::clock::now() - MTIME_SLACK;
    // the worker's own runs of the check, `rc check <id> --mid`, are counted a run
    reset_mid_checks(&acting, id);
    // the gotchas the run hits, for the journal and the summary
    let mut gotcha_hits: Vec<ra_gotchas::Hit> = Vec::new();
    // a run of a card resumes the session of the last one (the person: be generous with
    // --resume unless the topic changes), but a rerun of a card that is not absolute starts
    // fresh on the rerun block ([`rerun_resume`]); a stale session is dropped too, and the run
    // says so first
    let rerun = if is_rerun(&node, worker) && !lint_pass { rerun_block(&task, &node, &acting, &root) } else { None };
    // the environment this run has, read against by the next one's rerun block
    save_stamp(&root, id);
    // a run after an idle one starts fresh, as `--clear-session` would: the session that changed
    // nothing is only resumed to change nothing again at a heavier turn. Its prompt is the last
    // report and the review, not the transcript
    let idle = last_run(&task).and_then(idle_of).map(str::to_owned);
    let rerun = rerun.map(|block| match last_run(&task) {
        Some(last) if idle.is_some() => format!("{block}\nThe last run changed nothing; its report:\n{}\n", last.report.trim()),
        _ => block,
    });
    if idle.is_some() && node.session.is_some() {
        store.set_session(id, None, unix_now()).await?;
    }
    // an idle run is the reason a session is not resumed, ahead of the rerun's own fresh start
    // (#210): it names what went wrong, which "a rerun starts a fresh session" does not
    let resumed = match rerun_resume(&node, rerun.is_some(), unix_now()) {
        _ if lint_pass => Err(LINT_PASS_FRESH.to_owned()),
        Ok(Some(_)) | Err(_) if idle.is_some() && node.session.is_some() => {
            Err(format!("the last run changed nothing ({})", idle.as_deref().unwrap_or_default()))
        }
        other => other,
    };
    let (resume, fresh) = match resumed {
        Ok(Some(session)) => {
            let noted =
                rerun.as_ref().map(|_| "  rerun on the session: the verdict and the failing check ride as the new prompt".to_owned());
            (Some(session), noted)
        }
        Ok(None) => (
            None,
            rerun.as_ref().map(|_| "  fresh start: no session to resume; a rerun carries the verdict and the failing check".to_owned()),
        ),
        Err(reason) if lint_pass => (None, Some(format!("  {reason}; the warnings and the lines around them are the prompt"))),
        Err(reason) if rerun.is_some() => {
            (None, Some(format!("  fresh start: {reason}; the rerun carries the last report, the review points and the failures")))
        }
        Err(reason) => (None, Some(format!("  fresh start: {reason}; the brief and the handoffs go instead"))),
    };
    if let Some(fresh) = &fresh {
        tracing::info!(target: crate::trace::PROGRESS, "{}", fresh.trim_start());
    }
    if caps.doubled {
        tracing::info!(target: crate::trace::PROGRESS, "context cap doubled (read-in hit)");
    }
    // whether the report was asked for a second time, for the end line
    let mut asked_twice = false;
    // the handoffs' reports are the history a lint pass does without; the task keeps them for
    // the handoff this run records
    let shown = lint_pass.then(|| ra_context::TaskContext { handoffs: Vec::new(), ..task.clone() });
    let shown = shown.as_ref().unwrap_or(&task);
    let session = if provider.has_own_tools() {
        // a harness acts in the workspace by itself: one call carries the whole task, and the
        // harness holds it to the budget; its hooks learn the card from the environment
        begin_run(&acting, id, caps.limit());
        let answer = provider
            .complete(CompletionRequest {
                model: worker.model.clone(),
                messages: harness_messages(
                    shown,
                    node.acceptance.as_deref(),
                    rerun.as_deref(),
                    &project,
                    &root,
                    worker.debug,
                    crate::hub::manifest::load().as_ref(),
                ),
                temperature: Some(0.4),
                max_tokens: Some(4096),
                tools: Vec::new(),
                workspace,
                budget_usd: budget,
                resume,
                max_turns: caps.limit(),
                max_context: caps.context,
                effort: Some(effort.to_string()),
                read_only: false,
            })
            .await;
        match answer {
            Ok(mut answer) => {
                worker.record_run(Some(id), &answer.usage, answer.turns, answer.read_in).await;
                gotcha_hits.clone_from(&answer.gotchas);
                if let Some(session) = answer.session_id.as_deref() {
                    // kept on the node with the card's fingerprint: the next run of it resumes
                    // this session while the card reads the same
                    store.set_session(id, Some((session, &card_fingerprint(&node))), unix_now()).await?;
                }
                // a report that names no file and no result is not sent to the reviewer, who
                // would only send the card back for it (#47, #134): the session, which still
                // holds the work, is asked for the report once more, with the files git status
                // shows changed as the prompt, and that answer is the handoff, whatever it says
                if is_empty_handoff(&answer.text)
                    && let Some(session) = answer.session_id.clone()
                {
                    asked_twice = true;
                    let changed = run_changed(&acting, &dirty_before, &git_status(&acting).await, run_started);
                    tracing::info!(target: crate::trace::PROGRESS, "the report named no file and no result; asking the session again");
                    let again = provider
                        .complete(CompletionRequest {
                            model: worker.model.clone(),
                            messages: vec![Message::user(report_ask(&changed))],
                            temperature: Some(0.4),
                            max_tokens: Some(4096),
                            tools: Vec::new(),
                            workspace: workspace_dir.clone(),
                            budget_usd: budget,
                            resume: Some(session),
                            max_turns: Some(REPORT_TURNS),
                            max_context: None,
                            effort: Some(effort.to_string()),
                            read_only: false,
                        })
                        .await;
                    match again {
                        Ok(second) => {
                            worker.record(Some(id), &second.usage, second.turns).await;
                            gotcha_hits.extend(second.gotchas);
                            add_usage(&mut answer.usage, &second.usage);
                            answer.turns += second.turns;
                            answer.turn_usage.extend(second.turn_usage);
                            answer.reads.extend(second.reads);
                            answer.text = second.text;
                        }
                        // the first answer stands: the check and the review judge what landed
                        Err(e) => tracing::warn!("the second ask for the report of #{id} failed: {e}"),
                    }
                }
                Ok(Session {
                    text: answer.text,
                    usage: answer.usage,
                    calls: 0,
                    turns: answer.turns,
                    turn_usage: answer.turn_usage,
                    cap: caps.limit(),
                    capped: None,
                    biggest: read_sizes(workspace_dir.as_deref(), &answer.reads),
                    reads: answer.reads,
                    read_in: answer.read_in,
                    whole_reads: answer.whole_reads,
                })
            }
            // the harness reports its turns running out as an error, with what the run read,
            // wrote and cost: that is booked like a finished run's, so the card's cost counts
            // every session. When the project's cap is what ran out, the run ended as meant and
            // the card continues fresh
            Err(ProviderError::TurnsOut(answer)) => {
                worker.record_run(Some(id), &answer.usage, answer.turns, answer.read_in).await;
                gotcha_hits.clone_from(&answer.gotchas);
                match caps.restarts_at() {
                    Some(restart) => Ok(Session {
                        text: answer.text,
                        usage: answer.usage,
                        calls: 0,
                        turns: u64::from(restart),
                        turn_usage: answer.turn_usage,
                        cap: Some(restart),
                        capped: Some(Capped::Turns(restart)),
                        biggest: read_sizes(workspace_dir.as_deref(), &answer.reads),
                        reads: answer.reads,
                        read_in: answer.read_in,
                        whole_reads: answer.whole_reads,
                    }),
                    // the hard cap fails the run, but what the worker got to stays for the next
                    None => Err(anyhow::Error::new(CutOff {
                        cap: caps.limit().map_or_else(|| "the harness's turn cap".to_owned(), |limit| Capped::Turns(limit).to_string()),
                        said: answer.text.clone(),
                        failure: format!("{:#}", map_error(ProviderError::TurnsOut(answer))),
                    })),
                }
            }
            // the project's context cap ended the run after the turn that crossed it: booked
            // and continued like one the turn cap ended
            Err(ProviderError::ContextOut { completion: answer, cap, size }) => {
                worker.record_run(Some(id), &answer.usage, answer.turns, answer.read_in).await;
                gotcha_hits.clone_from(&answer.gotchas);
                Ok(Session {
                    text: answer.text,
                    usage: answer.usage,
                    calls: 0,
                    turns: answer.turns,
                    turn_usage: answer.turn_usage,
                    cap: caps.limit(),
                    capped: Some(Capped::Context { cap, size }),
                    biggest: read_sizes(workspace_dir.as_deref(), &answer.reads),
                    reads: answer.reads,
                    read_in: answer.read_in,
                    whole_reads: answer.whole_reads,
                })
            }
            Err(error) => Err(map_error(error)),
        }
    } else {
        let context = bare_context(shown, rerun.as_deref(), &project, worker.debug);
        tool_loop(provider.as_ref(), worker, (id, Some(store)), context, workspace.as_deref(), budget, caps).await
    };
    worker.end_run(id, end_reason(&session)).await;
    // the person took the card out of progress while this run went: the run ends here, whatever
    // it reached, and its state, notes and session are the person's, not a continuation's
    if let Some(state) = card_left(store, id).await {
        let line = stopped_line(id, &node.title, state);
        let usage = session.map(|s| s.usage).unwrap_or_default();
        return Ok((vec![line], usage, 0));
    }
    // the head of the handoff is the orchestrator's, not the worker's: where the tree is, the
    // branch's commits, what happened to it since the last run, and the acceptance's checks
    // still open, so the next run reads them as facts
    let tree = crate::review::tree_of(&acting, &base, worker.branch.as_deref(), since).await;
    let acceptance = named_checks(node.acceptance.as_deref());
    // the worker's mid-run checks are facts of the run, as the tree's lines are
    let mut mid_checks: String = mid_check_lines(&acting, id).join("\n");
    if !mid_checks.is_empty() {
        mid_checks.push('\n');
    }
    // the handoff of this run, for the next one of the card; the done cards' files go
    let handoff = |outcome: &str, check: &str, report: &str, changed: Vec<String>, open: &[String]| ra_context::Handoff {
        when: unix_now(),
        model: worker.model.clone(),
        outcome: outcome.to_owned(),
        check: check.to_owned(),
        verdict: None,
        changed,
        // the tree's head comes first, then the ladder's move, so the next run and the person
        // see where the tree is and why this model
        // the route rides in the head with the tree's lines, never ahead of the worker's report:
        // what reads a report reads how it opens (the context cap's words, the rerun's quote)
        // the worker's mid-run checks ride there too, or close the report when there is no head
        report: {
            let head = tree.head(open);
            let tail = if head.is_empty() && !mid_checks.is_empty() { format!("\n{}", mid_checks.trim_end()) } else { String::new() };
            format!(
                "{}{}{}{tail}",
                if head.is_empty() {
                    head
                } else {
                    format!("{head}{mid_checks}route: {} · {}\n{}", worker.route().name(), worker.model, tree.already())
                },
                if lint_pass { format!("{LINT_PASS_FRESH}.\n") } else { String::new() },
                worker.step.as_ref().map_or_else(|| report.to_owned(), |step| format!("Route: {step}.\n{report}"))
            )
        },
        warnings: Vec::new(),
        read: Vec::new(),
        failures: Vec::new(),
    };
    let mut session = match session {
        Ok(session) => session,
        Err(error) => {
            // a spent budget is worth a note: the next attempt sees what the last one cost
            if let Some(over) = error.downcast_ref::<OverBudget>() {
                store.set_notes(id, &over.to_string()).await?;
            }
            store.set_state_by(id, TaskState::Failed, Source::Run).await?;
            let cut = error.downcast_ref::<CutOff>();
            let said = cut.map_or("", |cut| cut.said.as_str());
            let changed = card_changed(&root, &acting, &dirty_before, &git_status(&acting).await, run_started, said, &task).await;
            // a run its turn cap ended before a report leaves the agent's own handoff: the
            // tree's changes and the worker's last words, so the next run starts from them
            let last = match cut {
                Some(cut) => {
                    let report = ra_context::cut_off_report(&cut.cap, &changed, &cut.said);
                    handoff(
                        ra_context::CUT_OFF,
                        &format!("no check: {} ended the run before its report", cut.cap),
                        &report,
                        changed,
                        &acceptance,
                    )
                }
                None => {
                    let why = format!("{error:#}");
                    // a worker that died leaves its exit code and last stderr lines on the card
                    if why.contains("\nstderr, last ") {
                        store.set_notes(id, &why).await?;
                    }
                    handoff("failed", "no check: the run failed", &why, changed, &acceptance)
                }
            };
            task.record(last);
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
    // the project's turn or context cap ended the run: no check, no verdict, the card stays in
    // progress with a handoff of what was done and what is left, and its session is dropped so
    // the next run, which the walk starts right away, opens on a small context again
    if let Some(cap) = session.capped {
        store.set_session(id, None, unix_now()).await?;
        let changed = card_changed(&root, &acting, &dirty_before, &git_status(&acting).await, run_started, &session.text, &task).await;
        // the checkpoint names what is left: the checks still open and the card's files no run
        // has touched yet, beside what this one changed
        let touched: Vec<&String> = task.handoffs.iter().flat_map(|h| h.changed.iter()).chain(changed.iter()).collect();
        let untouched: Vec<String> = node.files.iter().filter(|file| !touched.contains(file)).cloned().collect();
        let report = capped_report(cap, &session, &changed, &node, &acceptance, &untouched);
        // the reviewer's points stand over a capped run as over a failed check: the next
        // session answers them still
        match carried_review(node.notes.as_deref()) {
            Some(review) => store.set_notes(id, &format!("{review}\n\n{report}")).await?,
            None => store.set_notes(id, &report).await?,
        }
        let why = format!("{cap} ended the run; the card continues in a fresh session");
        let turns = session.cap.map_or_else(|| session.turns.to_string(), |limit| format!("{}/{limit}", session.turns));
        let mut last = handoff(ra_context::CONTINUES, &format!("no check: {why}"), &report, changed, &acceptance);
        if let Some(branch) = &worker.branch {
            last.check = format!("{} · branch {branch}", last.check);
        }
        // the regions this session read, which the fresh one's map lists instead of their being
        // read in again (#103's three sessions each began with the same twenty turns of reading)
        last.read = ra_context::read_set(&session.reads);
        task.record(last);
        save_context(store, &root, &task).await;
        let sizes = turn_sizes(&session);
        let mut lines = vec![format!(
            "↻ #{id} {} continues on {} · turns {turns}, {} tool call(s) · {} · {sizes} · {} · {why}",
            node.title,
            short(&worker.model),
            session.calls,
            tokens_line(&session),
            cost_text(&session.usage)
        )];
        lines.extend(fresh);
        lines.extend(noted);
        return Ok((lines, session.usage, gotcha_hits.len()));
    }
    // the local checks before the card counts as done: no model call, the acceptance names
    // them (else the project, else the workspace tests), and they run in order up to the first
    // failure
    let Checks { commands, lint: project_lint, timeout, orchestrated, .. } = checks_of(&root, &node);
    let command = commands.join(" · ");
    // a worktree's check builds in the target the worker built in, `target/wt-<id>` under the
    // project's, whatever the process's own `CARGO_TARGET_DIR`: the artifacts are warm there,
    // and the project's target is the one every tree's check would otherwise queue behind
    let own_target = workspace_dir.as_deref().and_then(ra_providers::claude_code::shared_target);
    let toolbox = worker.toolbox(workspace_dir.as_deref().unwrap_or(Path::new("."))).map(|toolbox| {
        toolbox.with_timeout(timeout).with_env(own_target.iter().map(|t| ("CARGO_TARGET_DIR".to_owned(), t.display().to_string())))
    });
    // a check is a build: it takes one of the project's build slots, as the workers' own
    // builds do, so six runs do not compile at once; a cargo check also takes the hub's lock
    // on a target another project may share, so two builds do not fight over the directory
    // the wait goes to the project's feed, never a file in the worktree the branch commits
    let _slot = if commands.is_empty() { None } else { hold_build(&root, &acting).await };
    // the lock is on the target the check builds in: a worktree's own under the project's,
    // not the project's (six asm checks on one lock waited for each other for nothing)
    let target = own_target.unwrap_or_else(|| crate::hub::cargo_target(&root));
    let waited = match commands.iter().find(|c| c.trim_start().starts_with("cargo")) {
        Some(_) => guard_target(&root, &acting, &target, timeout).await,
        None => Ok(None),
    };
    // a wait that ran out is the check's failure, named as the output a timed-out check leaves
    let (_target, mut checks) = match waited {
        Ok(guard) => {
            let checks = run_checks(&toolbox, &commands, &orchestrated).await;
            (guard, checks)
        }
        Err(text) => {
            let command = commands.iter().find(|c| c.trim_start().starts_with("cargo")).cloned().unwrap_or_default();
            tracing::info!(target: crate::trace::PROGRESS, "check failed: {command} · {text}");
            // not a setup failure: a rerun once the holder is done can move the card
            let run = CheckRun {
                command,
                passed: false,
                exit: text.clone(),
                first_error: text.clone(),
                text,
                failed_tests: 0,
                failures: Vec::new(),
                tests: Vec::new(),
                setup: false,
            };
            (None, vec![run])
        }
    };
    // a failure the base gives too is not the card's: in a card's worktree the failed tests run
    // alone on the project's base commit, and the failures the two share are marked, so a rerun
    // does not spend its turns on what it did not break. When every one fails there, main is
    // red, and the card waits for main to move instead of going back to its worker (507 asm
    // reruns hunted a defect of main's). A failure that names no test runs whole once more on
    // the project's tree
    let mut red_main = None;
    if worker.branch.is_some()
        && let Some(failed) = checks.iter_mut().find(|run| !run.passed && !run.tests.is_empty())
    {
        if let Some(base) = main_failures(worker, &root, &failed.command, &failed.tests, timeout).await {
            let shared: Vec<String> = failed
                .failures
                .iter()
                .map(|failure| failure.lines().next().unwrap_or_default().to_owned())
                .filter(|head| base.failing.contains(&run_name(head)))
                .collect();
            failed.failures = ra_context::marked_on_base(&failed.failures, &shared);
            if failed.tests.iter().all(|test| base.failing.contains(test)) {
                red_main = Some(base);
            }
        }
    } else if workspace_dir.is_some()
        && let Some(failed) = checks.iter_mut().find(|run| !run.passed && !run.failures.is_empty())
    {
        let base = base_failures(worker, &root, &failed.command, timeout).await;
        failed.failures = ra_context::marked_on_base(&failed.failures, &base);
    }
    // a test target the worktree lacks ran no test: the card that lists it is built on a main
    // that does not have it yet, so it waits for main's next commit and is no send-back
    let missing = checks.iter().find(|run| !run.passed).and_then(|run| ra_tools::missing_test_target(&run.command, &run.text));
    if worker.branch.is_some()
        && red_main.is_none()
        && missing.is_some()
        && let Ok(sha) = git(&root, &["rev-parse", "HEAD"]).await
    {
        red_main = Some(BaseRun { sha, failing: Vec::new() });
    }
    let status_after = git_status(&acting).await;
    // the card's own set: on a tree other hands share, a dirty file joins it only when the run
    // wrote to it and the report or an earlier handoff names it (#157 lost a named file, #185
    // was judged on the person's edits); in a worktree it is the whole branch
    let mut changed = card_changed(&root, &acting, &dirty_before, &status_after, run_started, &session.text, &task).await;
    // a fix pass edits the files the run before it dirtied, which `git status` shows no
    // differently: its change is the run's and its own together, so the lint reads the lines
    // it was sent to fix and the handoff still names them
    if worker.fix.is_some()
        && let Some(last) = task.handoffs.last()
    {
        changed.extend(last.changed.iter().cloned());
    }
    changed.sort();
    changed.dedup();
    // a failed check leads the notes and the handoff: the failed command with its tests and the
    // tail of its output first, then the commands that passed before it with their exit codes,
    // then the worker's answer, so the next run reads the failures before anything
    // a worker's own run of the check decides nothing: a report that says the check is still
    // running (a background shell the session outlived; #159 handed one off twice) is a
    // placeholder, and the agent's run, exit and clipped tail, stands in the notes and the
    // handoff where the claim was
    let claimed = without_check_claims(&session.text);
    let placeholder = claimed.is_some();
    if let Some(text) = claimed {
        session.text = text;
    }
    let mut report = session.text.clone();
    // what leads the answer in the notes and the handoff alike: the failed check, when one did,
    // else the agent's run of the checks when the worker's report claimed its own was running,
    // else the runs of the commands only the orchestrator ran, which the worker never saw
    let mut lead = String::new();
    let (mut passed, mut check_line) = match checks.iter().find(|run| !run.passed) {
        None => {
            let ran: Vec<String> = checks
                .iter()
                .filter(|run| placeholder || orchestrated.contains(&run.command))
                .map(|run| format!("check passed: {} · {}\n{}", run.command, run.exit, run.text))
                .collect();
            if !ran.is_empty() {
                lead = format!("{}\n\n", ran.join("\n\n"));
                report = format!("{lead}{}", session.text);
            }
            (true, format!("  check passed: {command}"))
        }
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
    // the failure's fingerprint rides on the check line, which the handoff keeps: the next run
    // of the card tells a failure that repeats on the same tree (asm #131 and #132 ran 23 times
    // on a check cargo rejected on its own arguments)
    let failed = checks.iter().find(|run| !run.passed);
    let print = match failed {
        Some(failed) => {
            let diff = git_text(&acting, &["diff", "HEAD"]).await;
            let base = git_text(&acting, &["rev-parse", "HEAD"]).await;
            Some(check_fingerprint(&failed.command, &failed.first_error, &diff, &base))
        }
        None => None,
    };
    if let Some(print) = &print {
        check_line = format!("{check_line}{}{print}", ra_context::FINGERPRINT_MARK);
    }
    let last_print = task
        .handoffs
        .iter()
        .rev()
        .find(|h| !h.outcome.starts_with("note from"))
        .and_then(ra_context::Handoff::fingerprint)
        .map(str::to_owned);
    // a card whose failed tests fail on main too waits for main's next commit: neither a rerun
    // nor a block for a person
    let red = red_main.as_ref().map(|base| match &missing {
        Some(target) if base.failing.is_empty() => format!("test target {target} not in the worktree"),
        _ => format!("main fails {} at {}", base.failing.join(", "), short_sha(&base.sha)),
    });
    let held = if red.is_some() { None } else { held_reason(&session.text, &changed, failed, print.as_deref(), last_print.as_deref()) };
    // the lint of what the run changed, once the check passed: the orchestrator finds the
    // warnings so the worker spends no turns on them. A lint that cannot run is a line, not a
    // verdict; a warning in a changed file is.
    let mut lint_lines = Vec::new();
    let mut lint_warnings = Vec::new();
    // a failed check gets its lint too, so one rerun meets both lists; a card waiting on main is
    // not sent back, and its lint would be spent on nothing
    let check_failed = !passed;
    if (passed || red.is_none())
        && let Ok(toolbox) = &toolbox
    {
        let base = diff_base(&root, &acting).await;
        let lint = lint_changed(toolbox, &acting, &changed, project_lint.as_deref(), &base).await;
        for skipped in &lint.skipped {
            lint_lines.push(format!("  lint skipped: {skipped}"));
        }
        if lint.fixed > 0 {
            lint_lines.push(format!("  lint: {} fixed by the machine", lint.fixed));
        }
        if lint.warnings.is_empty() {
            if !lint.command.is_empty() {
                lint_lines.push(format!("  lint clean: {}", lint.command));
            }
        } else {
            passed = false;
            if check_failed {
                report = format!("{report}\n\n{}", lint.warnings.join("\n\n"));
                check_line = format!(
                    "{check_line} · lint: {} warning(s) beside it from {} · the lines are in the notes",
                    lint.warnings.len(),
                    lint.command
                );
            } else {
                report = format!("{}\n\n{}", lint.warnings.join("\n\n"), session.text);
                check_line = format!(
                    "  check passed: {command} · lint: {} warning(s) in the changed files from {} · the lines are in the notes",
                    lint.warnings.len(),
                    lint.command
                );
            }
            // each point quotes its line, so a merge from main that moves it is followed by text
            lint_warnings = lint
                .warnings
                .iter()
                .map(|w| crate::commands::review::points::quote(w, |file| std::fs::read_to_string(acting.join(file)).ok()))
                .collect();
        }
    }
    // a card no rerun can move is held for a person on the first sight, with the line that
    // says why on its check line, the feed and the run's lines; the walk does not send it again.
    // A card that waits on main leads its notes with why, which the status reads and the
    // re-queue takes off again
    if let Some(reason) = &red {
        let mut preface = format!("{reason}; {RED_MAIN_REQUEUE}\n\n");
        if let Some(review) = carried_review(node.notes.as_deref()) {
            let _ = write!(preface, "{review}\n\n");
        }
        report = format!("{preface}{report}");
        store.set_notes(id, &report).await?;
        store.set_state_by(id, TaskState::Blocked, Source::Run).await?;
        if let Some(base) = &red_main {
            wait_on_main(&root, id, &base.sha);
        }
        check_line = format!("{check_line} · waits: {reason}");
    } else if let Some(reason) = &held {
        passed = false;
        if let Some(review) = carried_review(node.notes.as_deref()) {
            report = format!("{review}\n\n{report}");
        }
        store.set_notes(id, &with_left_points(node.notes.as_deref(), &report)).await?;
        store.set_state_by(id, TaskState::Blocked, Source::Run).await?;
        check_line = format!("{check_line} · blocked: {reason}");
        let text = format!("#{id} blocked: {reason}; a person sets it defined again");
        tracing::warn!("{text}");
        crate::pair::event(&root, &text);
    } else if passed {
        // the points that left the card stay on its notes, so its next reviewer does not raise
        // them again
        store.set_notes(id, &with_left_points(node.notes.as_deref(), &report)).await?;
        store.set_state_by(id, TaskState::Implemented, Source::Run).await?;
    } else {
        // a rerun that fails its check leaves no new verdict, so the reviewer's points stay at
        // the head of the notes until a reviewer judges the card again: the next rerun reads
        // them there, not off the roadmap
        if let Some(review) = carried_review(node.notes.as_deref()) {
            report = format!("{review}\n\n{report}");
        }
        let kept = with_left_points(node.notes.as_deref(), &report);
        if kept != session.text {
            store.set_notes(id, &kept).await?;
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
    let handed = if lint_warnings.is_empty() || check_failed { format!("{lead}{answer}") } else { answer.to_owned() };
    let open: Vec<String> =
        acceptance.iter().filter(|item| !checks.iter().any(|run| run.passed && &run.command == *item)).cloned().collect();
    let outcome = if red.is_some() {
        "waits on main"
    } else if held.is_some() {
        "blocked"
    } else if passed {
        "implemented"
    } else {
        "needs revision"
    };
    // the checks' output, for the reviewer: it reads what the check showed, not only its line
    let output: Vec<String> = checks.iter().map(|run| format!("$ {} · {}\n{}", run.command, run.exit, run.text.trim_end())).collect();
    let tip = match &worker.workspace {
        Some(path) if !same_tree(&root, path) => content_tip(path).await,
        _ => None,
    };
    task.set_check_output(&stamped(&output.join("\n\n"), tip.as_deref()));
    let mut last = handoff(outcome, check_line.trim(), &handed, changed, &open);
    last.warnings = lint_warnings;
    // a run sent back by its check hands on what it read, as a capped one does, and every
    // failure of the check whole, which the clipped output in the notes may not hold
    if !passed {
        last.read = ra_context::read_set(&session.reads);
        last.failures = checks.iter().find(|run| !run.passed).map(|run| run.failures.clone()).unwrap_or_default();
    }
    task.record(last);
    save_context(store, &root, &task).await;
    let turns = match session.cap {
        Some(cap) => format!("{}/{cap}", session.turns),
        None => session.turns.to_string(),
    };
    let mut lines = vec![
        format!(
            "{} #{} {} {} on {} · turns {turns}, {} tool call(s) · {} · {sizes} · {} · {verdict}{}{}",
            if passed { "✓" } else { "✗" },
            id,
            node.title,
            outcome,
            short(&worker.model),
            session.calls,
            tokens_line(&session),
            cost_text(&session.usage),
            if asked_twice { " · report asked twice" } else { "" },
            if lint_pass { format!(" · {LINT_PASS_FRESH}") } else { String::new() }
        ),
        check_line,
    ];
    if let Some(reason) = &red {
        lines.push(format!("  #{id} waits: {reason}; {RED_MAIN_REQUEUE}"));
    }
    if let Some(reason) = &held {
        lines.push(format!("  #{id} blocked: {reason}; a person sets it defined again"));
    }
    lines.extend(fresh);
    lines.extend(noted);
    lines.extend(lint_lines);
    // a run whose mean turn passed the ledger's ceiling names its biggest tool result, for the
    // next walk to make that read or command smaller
    let per_turn = average_turn(&session);
    if per_turn > crate::retro::TURN_CEILING {
        match crate::retro::record_heavy(&root, id, per_turn, &session.biggest) {
            Ok(text) => lines.push(format!("  ⚠ {text}")),
            Err(e) => tracing::warn!("the weight of #{id} was not noted: {e:#}"),
        }
    }
    // in debug mode the files the run read again, the context every later turn carried, and
    // the worker's own look back: kept for the next runs, and named here
    if worker.debug {
        if let Some(again) = reread(&session.reads) {
            lines.push(format!("  re-read: {again}"));
        }
        let mut lookback_notes = crate::retro::kept(crate::retro::parse(&session.text));
        // a bullet without its tag is put to the classify role, when it has a model: the
        // worker forgets the tag more often than it misplaces it
        for note in lookback_notes.iter_mut().filter(|note| !note.machine) {
            if let Some(answer) = worker.ask_role(roles, Role::Classify, Some(id), crate::retro::CLASSIFY, &note.text).await
                && let Some(machine) = crate::retro::classified(&answer)
            {
                note.machine = machine;
            }
        }
        // the acceptance the notes were written against, so a later change of it retires them
        let print = crate::retro::acceptance_print(node.acceptance.as_deref());
        for note in &mut lookback_notes {
            note.against = Some(print.clone());
        }
        if let Err(e) = crate::retro::record(&root, Some(id), &lookback_notes) {
            tracing::warn!("the look back of #{id} was not kept: {e:#}");
        }
        for note in &lookback_notes {
            lines.push(format!("  ↯ {}{}", if note.machine { "[machine] " } else { "" }, note.text));
        }
    }
    lines.push("  the full answer is in the notes of the card in rhipper.md".to_owned());
    if !gotcha_hits.is_empty() {
        let mut ids: Vec<&str> = gotcha_hits.iter().map(|h| h.id.as_str()).collect();
        ids.dedup();
        lines.push(format!("  ⚠ {} gotcha(s): {} · rc gotchas lists them", gotcha_hits.len(), ids.join(", ")));
    }
    Ok((lines, session.usage, gotcha_hits.len()))
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

/// The caps of a run; no turn cap stands by default, and the context cap is the project's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Caps {
    /// The hard cap, when `--turns` named one: a run that reaches it failed.
    hard: Option<u32>,
    /// The project's `[worker] turns`, when it names one: the run ends there with a handoff and
    /// the card continues in a fresh session.
    restart: Option<u32>,
    /// The project's `[worker] context`: a harness run ends the same way after a turn whose
    /// input passes this many tokens; `None`, the default, is no cap.
    context: Option<u64>,
    /// Whether `context` is twice the project's: the run is exempt from the plain cap, bounded.
    doubled: bool,
}

/// What ended a run short of its answer so the card continues in a fresh session: the project's
/// turn cap, or its context cap at the size of the turn that crossed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Capped {
    Turns(u32),
    Context { cap: u64, size: u64 },
}

impl std::fmt::Display for Capped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Turns(cap) => write!(f, "the turn cap of {cap}"),
            Self::Context { cap, size } => write!(f, "the context cap of {} tokens (a turn of {})", kilo(*cap), kilo(*size)),
        }
    }
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
    // the commands of the acceptance the worker may not run, or that outlive its turn, are
    // named under the card, so the session spends no turn on a refusal or a wait
    task.brief = format!("{}{}", brief.text, orchestrated_line(&checks_of(&root, node)));
    if let Some(last) = task.handoffs.last()
        && let Some(turns) = worker.last_run_turns(node.id).await
        && let Some(note) = short_run_note(last, turns)
    {
        task.brief = format!("{}\n\n{note}", task.brief);
    }
    let acts_in = workspace.as_deref().unwrap_or(&root);
    // a card that continues or came back already has files in its tree
    if (task.handoffs.last().is_some_and(|last| last.outcome == ra_context::CONTINUES) || sent_back(node))
        && let Some(note) = prior_files_note(acts_in, "main").await
    {
        task.brief = format!("{}\n\n{note}", task.brief);
    }
    // the outline is read from the tree the worker acts in, as its quotes are, so a rerun in a
    // worktree gets the line numbers the file has there
    let changed = last_change(acts_in, node, &task).await;
    // a card sent back by its reviewer carries the verdict's points in its context: the notes
    // that hold them are in the roadmap, which the worker does not read
    task.review = if is_rerun(node, worker) { ra_context::review_of(node.notes.as_deref().unwrap_or_default()) } else { None };
    task.card_print = Some(card_print(node));
    // the map of a rerun quotes what its failures, warnings and points name and lists what the
    // last session read as rows, so the worker opens on the few items the rerun is about
    let points = rerun_points(task.review.as_deref(), task.handoffs.last());
    let read = task.handoffs.last().map_or(&[][..], |last| last.read.as_slice());
    task.outlines = map_of_card(acts_in, node, &brief.files, &changed, &points, read);
    let project =
        if draft { ra_context::Project::ensure(&root).unwrap_or_default() } else { ra_context::Project::load(&root).unwrap_or_default() };
    // the context rides cut to the card's crates (whole on rhipper-asm it is 12k tokens a run);
    // the active gambits ride after it as the person's standing direction, and the sources git
    // does not track yet under the card's crates after that: a card's outline cannot name a
    // file the tree does not show
    let project = ra_context::cut_to(&project, &brief.files);
    let project = ra_context::with_direction(&project, &direction_lines(&store.gambits().await?));
    let project = ra_context::with_untracked(&project, &ra_context::untracked_sources(&root, &brief.files));
    // an explicit budget, else the complexity's while budgets are on, else none at all; a turn
    // cap only when one was asked for. No limit stands by default: the person asked for none
    let budget = worker.budget.or_else(|| worker.budgets.then(|| budget_for(node.complexity)));
    let router = crate::router::Router::load(&root).ok();
    // an absolute card works until it is done: no cap restarts it or ends it, the whole run in
    // one session costing less than the restarts a cap would cut it into
    let absolute = node.is_absolute();
    let configured = router.as_ref().and_then(|router| router.worker.context());
    let doubled = !is_lint_pass(worker) && (capped_reading_in(&task) || absolute);
    let caps = Caps {
        hard: worker.turns.filter(|_| !absolute),
        restart: router.as_ref().and_then(|router| router.worker.turns()).filter(|_| !absolute),
        // twice the context cap on a run whose last session hit it while still reading in: the
        // fresh session reads the same files and hits it again, and the card never gets past
        // them (runelite #15: AffineTransform and Arc2D from the JDK, 161k at turn 7); never
        // none, or the session grows without bound
        context: configured.map(|c| if doubled { c.saturating_mul(2) } else { c }),
        doubled: doubled && configured.is_some(),
    };
    let effort = worker.effort.unwrap_or(node.effort);
    Ok(Prepared { workspace, root, task, project, budget, caps, effort })
}

/// The messages of a harness run: the worker's role, the project context and the machine notes
/// as system messages, the task as the user turn. The machine notes name what tripped the last
/// runs, so the worker does not find out the hard way again, and the warnings the last run of
/// the card left, so a second attempt opens on the exact lines; the machine's `manifest` gives
/// the lines of the tools the card names, so the worker does not go looking for them. On a
/// rerun the `rerun` block stands in for the handoffs. `acceptance` is the card's, which the
/// notes are judged against.
pub(crate) fn harness_messages(
    task: &ra_context::TaskContext,
    acceptance: Option<&str>,
    rerun: Option<&str>,
    project: &str,
    root: &Path,
    debug: bool,
    manifest: Option<&crate::hub::manifest::Manifest>,
) -> Vec<Message> {
    let mut messages = vec![Message::system(SYSTEM)];
    if !project.is_empty() {
        messages.push(Message::system(format!("The project, from {}:\n{project}", ra_context::PROJECT_FILE)));
    }
    let tools = manifest.map(|m| m.block_for(&task.brief)).unwrap_or_default();
    let notes = [
        BUILD_RULES.to_owned(),
        machine_notes(root, project, (task.id, acceptance)),
        tools,
        warnings_note(task),
        ra_context::lint_primer(root),
        digest_block(&crate::hub::digest_lines(root)),
    ]
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
    // the change's commit message, which the landing squashes the card under
    messages.push(Message::system(COMMIT_PROMPT));
    // a project whose builds are the orchestrator's alone says so up front, so the worker does
    // not spend a turn finding the build refused
    // the notes of the worktree are the card's tree's: `root` is the project's, whose target is
    // the main one, so a seed from it copied nothing and a worktree kept the binaries it was
    // made with (asm #238: only an rc.exe, hours after ras.exe was built into the main target)
    let tree = card_tree(root, task.id);
    if crate::router::Router::load(root).is_ok_and(|r| r.worker.builds() == 0) {
        messages.push(Message::system(NO_BUILDS));
        // the seed runs at every run's start, so a binary built into the main target since the
        // worktree was made reaches it
        let binaries = ra_providers::claude_code::worktree_prebuilt_note(&tree);
        if !binaries.is_empty() {
            messages.push(Message::system(binaries));
        }
    }
    let dirs = ra_providers::claude_code::session_dirs_note(&tree);
    if !dirs.is_empty() {
        messages.push(Message::system(dirs));
    }
    // the investigator's brief, when one went ahead: where the code is and what to change,
    // so the builder writes rather than reads
    let mut text = task_text(task, rerun);
    if let Some(brief) = research_brief(root, task.id) {
        let _ = write!(
            text,
            "\n\nAn investigator read the tree for this card before you and left this brief; open on it, check what you rely on, and do not read the tree over again where it already says:\n\n{brief}"
        );
    }
    messages.push(Message::user(text));
    messages
}

/// The task as the worker reads it: the brief, the outlines and the last handoffs, or on a
/// rerun the brief, the outlines and the `rerun` block in place of the handoffs, whose reports
/// are the transcript the rerun does not pay for.
fn task_text(task: &ra_context::TaskContext, rerun: Option<&str>) -> String {
    match rerun {
        None => task.prompt(),
        Some(block) => {
            // the review is judged stale or not on the handoffs before they leave the prompt:
            // the newest one's verdict says which words the review judged
            let bare = ra_context::TaskContext { handoffs: Vec::new(), review: task.review_shown(), ..task.clone() };
            format!("{}\n{block}", bare.prompt())
        }
    }
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
fn bare_context(task: &ra_context::TaskContext, rerun: Option<&str>, project: &str, debug: bool) -> String {
    // the rerun block carries the last lint's warnings, which the handoffs a rerun leaves out held
    let context = task_text(task, rerun);
    let mut text = if project.is_empty() { context } else { format!("The project:\n{project}\n\n{context}") };
    if debug {
        text.push_str("\n\n");
        text.push_str(crate::retro::PROMPT);
    }
    text.push_str("\n\n");
    text.push_str(COMMIT_PROMPT);
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
    let route = if worker.routed {
        let root = workspace_of(store);
        let router = crate::router::Router::load(&root)?;
        let (choice, step) = router.route(&node, crate::router::IMPLEMENTER, &worker.model, rejections(&root, id));
        let step = step.map_or_else(String::new, |step| format!(" · {}", step.note));
        Some(format!("  route {} {}{step}", resolve(&choice.model), router.reason(&node, &choice)))
    } else {
        None
    };
    let worker = &worker.for_card(&workspace_of(store), &node)?;
    // a dry run calls no model, so it needs no key: without one (CI has none) the route alone
    // says whether the run is a harness, and the harness's command line is left out
    let provider = worker.provider().ok();
    let Prepared { workspace, root, task, project, budget, caps, effort } = prepare(store, &node, worker, false).await?;
    let harness = provider.as_ref().map_or(worker.route() == Route::ClaudeCode, |p| p.has_own_tools());
    let checks = checks_of(&root, &node);
    let acting = workspace.clone().unwrap_or_else(|| root.clone());
    let rerun = if is_rerun(&node, worker) { rerun_block(&task, &node, &acting, &root) } else { None };
    let resume = rerun_resume(&node, rerun.is_some(), unix_now());
    let messages = if harness {
        harness_messages(
            &task,
            node.acceptance.as_deref(),
            rerun.as_deref(),
            &project,
            &root,
            worker.debug,
            crate::hub::manifest::load().as_ref(),
        )
    } else {
        vec![Message::system(SYSTEM), Message::user(bare_context(&task, rerun.as_deref(), &project, worker.debug))]
    };
    let request = CompletionRequest {
        model: worker.model.clone(),
        messages,
        temperature: Some(if harness { 0.4 } else { 0.2 }),
        max_tokens: Some(4096),
        tools: if harness { Vec::new() } else { ra_tools::Toolbox::specs() },
        workspace,
        budget_usd: budget,
        resume: resume.clone().unwrap_or_default(),
        max_turns: caps.limit(),
        max_context: caps.context,
        effort: Some(effort.to_string()),
        read_only: false,
    };
    let mut lines = vec![
        format!(
            "dry run of #{id} {} on {} via {}: nothing is called, the card is as it was",
            node.title,
            worker.model,
            worker.route().name()
        ),
        format!(
            "  budget {} · turn cap {}{}{} · resume {} · {}",
            budget.map_or_else(|| "none".to_owned(), |b| format!("${b:.2}")),
            caps.hard.map_or_else(|| "none".to_owned(), |c| c.to_string()),
            caps.restart.map_or_else(String::new, |r| format!(" · fresh session after {r} turns")),
            caps.context.map_or_else(String::new, |c| format!(
                " · fresh session past {} tokens a turn{}",
                kilo(c),
                if caps.doubled { " · context cap doubled (read-in hit)" } else { "" }
            )),
            match resume {
                Ok(Some(session)) => session,
                Ok(None) => "none".to_owned(),
                Err(reason) => format!("none, {reason}"),
            },
            checks.describe()
        ),
    ];
    lines.extend(route);
    let always = landing_commands(&root, Vec::new(), &[]);
    if !always.is_empty() {
        lines.push(format!("  every landing's merged tree also runs: {}", always.join(" · ")));
    }
    // what the pre hook makes of a check when the worker runs it itself: the command as it
    // runs, with its tail, shown before a run does it
    if let Ok(catalogue) = Catalogue::load(&root.join(GOTCHAS_FILE)) {
        for check in &checks.commands {
            if let ra_gotchas::Verdict::Rewrite { command, .. } = catalogue.command(check, ra_gotchas::Conditions::default()) {
                lines.push(format!("  the pre hook runs `{check}` as `{command}`"));
            }
        }
    }
    let command = provider.as_ref().and_then(|p| p.command_line(&request));
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
    /// The fixes the machine applied before the lint judged.
    fixed: usize,
    /// The lints that could not run, each with its reason.
    skipped: Vec<String>,
}

/// Lints what `changed` touched and keeps the diagnostics that point into the lines the change
/// touched (`git diff -U0` of those files; a file git does not know is new, and all of it
/// counts), not the warnings its files had before. Without a `project` lint, clippy runs over
/// each crate that owns a changed file; an empty one is no lint at all; any other is the
/// project's command, run as a check is, outside the shell allowlist. Clippy exits 0 on a
/// warning, so the output decides, not the exit code. `base` is what the card's lines are read
/// against: `HEAD` in the project's own tree, the merge base in a worktree, so the card's
/// committed checkpoints count as its lines and the lines the catch-up merge brought in do not.
async fn lint_changed(toolbox: &ra_tools::Toolbox, root: &Path, changed: &[String], project: Option<&str>, base: &str) -> Lint {
    let mut crates: Vec<String> = Vec::new();
    let command = match project {
        Some("") => return Lint::default(),
        Some(command) if !changed.is_empty() => command.to_owned(),
        Some(_) => return Lint::default(),
        None => {
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
    let diff = match toolbox.check(&format!("git diff -U0 {base} -- {}", changed.join(" "))).await {
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
    // what clippy marks machine-applicable inside the card's lines is written by the machine, then
    // formatted; the lint below judges what is left, with no model and no turn
    if !crates.is_empty() {
        let json = format!("{} --message-format=json", lint.command);
        if let Ok((_, output)) = toolbox.check(&json).await {
            let fixes = ra_check::fixes::fixes(&output, &ranges);
            if let Ok(applied) = ra_check::fixes::apply(root, &fixes)
                && applied > 0
            {
                lint.fixed = applied;
                let _ = toolbox.check(&format!("cargo fmt -p {}", crates.join(" -p "))).await;
            }
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
        let _ = ra_context::TaskContext::prune(root, &open);
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
    // the points of a review rank the map in `prepare`, so the preview ranks it by them too;
    // a fix pass is the one rerun it cannot tell, having no worker
    task.review = if sent_back(&node) { ra_context::review_of(node.notes.as_deref().unwrap_or_default()) } else { None };
    task.card_print = Some(card_print(&node));
    let points = rerun_points(task.review.as_deref(), task.handoffs.last());
    let changed = last_change(&root, &node, &task).await;
    let read = task.handoffs.last().map_or(&[][..], |last| last.read.as_slice());
    task.outlines = map_of_card(&root, &node, &brief.files, &changed, &points, read);
    Ok(task.prompt())
}

/// The map of the card's files and of those its last session `read` as the prompt carries it:
/// the items its title, objective, acceptance and `points` point at, read from the tree the
/// worker acts in, where in its tracked sources a name the card's files do not define is, and
/// where the card's test goes.
fn map_of_card(
    root: &Path,
    node: &Node,
    files: &[String],
    changed: &ra_context::ChangedLines,
    points: &[String],
    read: &[String],
) -> String {
    let card =
        format!("{}\n{}\n{}", node.title, node.objective.as_deref().unwrap_or_default(), node.acceptance.as_deref().unwrap_or_default());
    let sources = ra_context::map::tracked_sources(root);
    let ask = ra_context::map::Ask { card: &card, points, read, sources: &sources };
    let mut files = files.to_vec();
    for entry in read {
        let (path, _) = ra_context::read_entry(entry);
        if !files.iter().any(|f| f.replace('\\', "/") == path.replace('\\', "/")) {
            files.push(path.to_owned());
        }
    }
    let text = |file: &str| std::fs::read_to_string(root.join(file)).ok();
    let mut map = ra_context::map::map_of(text, &files, &ask, changed, ra_context::MAP_ROOM).render(text, ra_context::QUOTE_ROOM);
    map.push_str(&ra_context::map::test_recipe(text, &files, &ask));
    map.push_str(&ra_context::large_files_note(root, &files));
    map
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
    let project = ra_context::with_direction(&project_context(store), &direction_lines(&store.gambits().await?));
    Ok(ra_context::with_untracked(&project, &ra_context::untracked_sources(&workspace_of(store), &[])))
}

/// The workspace: the directory of the roadmap file.
pub fn workspace_of(store: &Store) -> PathBuf {
    store.path().parent().filter(|p| !p.as_os_str().is_empty()).map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// How far before the run's start a file's modification time still counts as the run's: NTFS
/// stamps a write off the system tick (about 16 ms), and a write right after the start read as
/// before it, so the run's own file dropped out of its handoff.
const MTIME_SLACK: std::time::Duration = std::time::Duration::from_secs(1);

/// The files a run changed: the ones whose status line is new since the run began, and the
/// dirty ones it wrote to in the same time, by their modification time. A file another card
/// had already left dirty keeps its status line when this run edits it again, and the status
/// difference alone missed it: the reviewer then judged the card on a diff without its main
/// file (#152).
fn run_changed(root: &Path, before: &str, after: &str, since: std::time::SystemTime) -> Vec<String> {
    let mut changed = ra_context::changed_between(before, after);
    for line in after.lines() {
        let Some(path) = line.get(3..) else { continue };
        let path = path.rsplit(" -> ").next().unwrap_or(path).trim_matches('"');
        if changed.iter().any(|c| c == path) {
            continue;
        }
        let touched = std::fs::metadata(root.join(path)).and_then(|m| m.modified()).is_ok_and(|m| m >= since);
        if touched {
            changed.push(path.to_owned());
        }
    }
    changed.sort();
    changed.dedup();
    changed
}

/// The card's change, which the handoff's `changed` line names and the reviewer judges. In a
/// card's worktree the branch's files from its merge base ([`branch_files`]): every run of the
/// card and no one else's edit; a run that moved nothing names nothing, so the walk still
/// counts it idle. In the project's own tree what the run can be held to: a file whose status
/// line is new since the run began, or one already dirty that was written to inside the run
/// and that the report or an earlier handoff of the card names. A file dirty before the run
/// that only its time ties to it is not the card's: the person's edits in the shared checkout
/// sent #185 back three times.
async fn card_changed(
    project: &Path,
    acting: &Path,
    before: &str,
    after: &str,
    since: std::time::SystemTime,
    report: &str,
    task: &ra_context::TaskContext,
) -> Vec<String> {
    if !same_tree(project, acting) {
        return if run_changed(acting, before, after, since).is_empty() { Vec::new() } else { branch_files(project, acting).await };
    }
    let mut changed = ra_context::changed_between(before, after);
    let reported = reported_files(acting, report, after);
    let named = |path: &String| reported.contains(path) || task.handoffs.iter().any(|h| h.changed.contains(path));
    for path in dirty_paths(after) {
        if changed.contains(&path) || !named(&path) {
            continue;
        }
        if std::fs::metadata(acting.join(&path)).and_then(|m| m.modified()).is_ok_and(|m| m >= since) {
            changed.push(path);
        }
    }
    changed.sort();
    changed.dedup();
    changed
}

/// The paths `git status --porcelain` lists, a rename by its new name.
fn dirty_paths(status: &str) -> Vec<String> {
    status
        .lines()
        .filter_map(|line| line.get(3..))
        .map(|path| path.rsplit(" -> ").next().unwrap_or(path).trim_matches('"').to_owned())
        .collect()
}

/// The files a worker's report names in backticks that are in the tree and that `status`
/// (`git status --porcelain`) lists as dirty or new: what the run says it changed, held to
/// what git can show of it. A clean file would add nothing to the reviewer's diff and would
/// read to the lint as new, so it stays out; a span with spaces or a line of a command is not
/// a path.
fn reported_files(root: &Path, report: &str, status: &str) -> Vec<String> {
    let dirty = dirty_paths(status);
    let mut files: Vec<String> = report
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::trim)
        .filter(|span| !span.is_empty() && !span.contains(char::is_whitespace))
        .map(|span| span.trim_start_matches("./").replace('\\', "/"))
        .filter(|path| root.join(path).is_file() && dirty.iter().any(|d| d == path))
        .collect();
    files.sort();
    files.dedup();
    files
}

/// `git status --porcelain` of `root`, or nothing where git has nothing to say.
async fn git_status(root: &Path) -> String {
    match tokio::process::Command::new("git").args(["status", "--porcelain"]).current_dir(root).output().await {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).into_owned(),
        _ => String::new(),
    }
}

pub(crate) fn unix_now() -> u64 {
    crate::clock::unix_now()
}

/// The check of a node when its acceptance names none and its files name no crate.
const DEFAULT_CHECK: &str = ra_roadmap::check::WORKSPACE_TESTS;

/// The checks of a node: every backticked span of its acceptance that reads as a command line,
/// in the order written, else the tests of the whole workspace. A span is a command when its
/// first token is a build or script runner (cargo, python, npm, gradle and the like) or a
/// relative script path (`./gw.sh`, `.\gw.ps1`, `tools/run.py`); a bare path, an identifier or a
/// `git log` style reference is not.
#[must_use]
pub fn check_commands(acceptance: Option<&str>) -> Vec<String> {
    let commands = named_checks(acceptance);
    if commands.is_empty() { vec![DEFAULT_CHECK.to_owned()] } else { commands }
}

/// The command lines an acceptance names, possibly none.
fn named_checks(acceptance: Option<&str>) -> Vec<String> {
    acceptance
        .map(|text| text.split('`').skip(1).step_by(2).map(str::trim).filter(|span| is_command(span)).map(str::to_owned).collect())
        .unwrap_or_default()
}

/// What a card is judged by in its project: the checks and the lint.
struct Checks {
    /// The card's own backticked commands, else the project's `check`, else the workspace tests.
    commands: Vec<String>,
    /// The project's `lint` as the `[worker]` table has it; `None` leaves clippy in place.
    lint: Option<String>,
    /// How long each check, and the lint, may run: the project's `check_timeout`.
    timeout: std::time::Duration,
    /// The commands among them that are the orchestrator's alone ([`orchestrated_checks`]):
    /// their output rides the handoff, and the worker's prompt names them.
    orchestrated: Vec<String>,
    /// Whether the card's acceptance names a command; the start line says so when it does not.
    named: bool,
}

impl Checks {
    /// One line naming both, for the dry run header, the activity log and the pair log; the
    /// time limit only when the project set its own.
    fn describe(&self) -> String {
        let lint = match self.lint.as_deref() {
            None => "clippy over the changed crates",
            Some("") => "none",
            Some(command) => command,
        };
        let limit = if self.timeout == CHECK_TIMEOUT { String::new() } else { format!(" · {} s", self.timeout.as_secs()) };
        let source = if self.named { "" } else { "project default " };
        format!("check: {source}{}{limit} · lint: {lint}", self.commands.join(" · then "))
    }
}

/// The checks of `node` under the `[worker]` table of the project at `root`; a table that will
/// not parse is the default one, as the run already said so when it picked the model. Without a
/// named check the card's files pick the crates to test: a full workspace run outlives a session
/// on a large workspace.
fn checks_of(root: &Path, node: &Node) -> Checks {
    let worker = crate::router::Router::load(root).map(|router| router.worker).unwrap_or_default();
    let own = named_checks(node.acceptance.as_deref());
    let named = !own.is_empty();
    let commands = if named {
        own
    } else if let Some(check) = worker.check() {
        vec![check.to_owned()]
    } else {
        ra_roadmap::check::commands(&node.files)
    };
    let orchestrated = orchestrated_checks(node.acceptance.as_deref(), &worker);
    let timeout = card_check_timeout::card_check_timeout(&[node.objective.as_deref(), node.acceptance.as_deref()])
        .unwrap_or_else(|| worker.check_timeout());
    Checks { commands, lint: worker.lint().map(str::to_owned), timeout, orchestrated, named }
}

/// The words of a dry run of `command`, which the tool answers without running the check: cargo
/// tests with `--no-run`, other cargo commands and the build tools with `--help` (a rejected
/// argument fails before the help is printed), gradle with `--dry-run`, a python script parsed.
/// `None` for a command no dry run is safe for: a script, or an interpreter that would run it.
fn dry_words(command: &str) -> Option<Vec<String>> {
    // an interpreter given `--help` after its script would run the script
    const INTERPRETERS: [&str; 7] = ["bash", "node", "java", "ruby", "php", "deno", "bun"];
    let mut words = ra_tools::words(command).ok()?;
    let program = words.first()?.clone();
    // what follows `--` is the test binary's, not the tool's
    let stop = words.iter().position(|w| w == "--").unwrap_or(words.len());
    match program.as_str() {
        "cargo" => {
            let tests = words.get(1).is_some_and(|w| matches!(w.as_str(), "test" | "t" | "bench"));
            words.insert(stop, if tests { "--no-run" } else { "--help" }.to_owned());
        }
        "gradle" | "gradlew" => words.push("--dry-run".to_owned()),
        "python" | "python3" | "py" => {
            let script = words.get(1).filter(|w| Path::new(w).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("py")))?.clone();
            return Some(vec![program, "-m".to_owned(), "py_compile".to_owned(), script]);
        }
        other if ra_roadmap::check::CHECK_PROGRAMS.contains(&other) && !INTERPRETERS.contains(&other) => {
            words.insert(stop, "--help".to_owned());
        }
        _ => return None,
    }
    Some(words)
}

/// Whether a cargo `command` names, by `-p`/`--package` or `--test`, a package or test file that
/// the card's `+` entries in `files` create: the base tree cannot have it, so the dry run would
/// reject what the card is there to make (asm #180).
fn names_created(command: &str, files: &[String]) -> bool {
    let Ok(words) = ra_tools::words(command) else { return false };
    if words.first().map(String::as_str) != Some("cargo") {
        return false;
    }
    let created: Vec<&str> = files.iter().filter_map(|f| f.trim().strip_prefix('+')).map(|f| f.trim().trim_end_matches('/')).collect();
    let stop = words.iter().position(|w| w == "--").unwrap_or(words.len());
    let mut args = words[..stop].iter();
    while let Some(word) = args.next() {
        let (flag, inline) = match word.split_once('=') {
            Some((flag, value)) => (flag, Some(value)),
            None => (word.as_str(), None),
        };
        if !matches!(flag, "-p" | "--package" | "--test") {
            continue;
        }
        let Some(value) = inline.or_else(|| args.next().map(String::as_str)) else { break };
        let hit = if flag == "--test" {
            created.iter().any(|f| f.rsplit_once("/tests/").is_some_and(|(_, name)| name.strip_suffix(".rs") == Some(value)))
        } else {
            created.iter().any(|f| f.strip_suffix("/Cargo.toml").is_some_and(|dir| dir.rsplit('/').next() == Some(value)))
        };
        if hit {
            return true;
        }
    }
    false
}

/// Whether a cargo `command` names a package by `-p`/`--package` whose directory `<dir>/<pkg>/`
/// does not exist in `root` yet while a file of the card lies under it: a card that makes a new
/// crate names its sources but not a `+` Cargo.toml (asm #143 and #80).
#[must_use]
pub fn names_new_package(root: &Path, command: &str, files: &[String]) -> bool {
    let Ok(words) = ra_tools::words(command) else { return false };
    if words.first().map(String::as_str) != Some("cargo") {
        return false;
    }
    let stop = words.iter().position(|w| w == "--").unwrap_or(words.len());
    let mut packages = Vec::new();
    let mut args = words[..stop].iter();
    while let Some(word) = args.next() {
        match word.as_str() {
            "-p" | "--package" => packages.extend(args.next().map(String::as_str)),
            other => packages.extend(other.strip_prefix("--package=")),
        }
    }
    let paths: Vec<String> = files.iter().map(|f| f.trim().trim_start_matches('+').trim().replace('\\', "/")).collect();
    packages.iter().any(|pkg| {
        paths.iter().any(|path| {
            let parts: Vec<&str> = path.split('/').collect();
            parts
                .iter()
                .enumerate()
                .take(parts.len().saturating_sub(1))
                .any(|(at, part)| part == pkg && at > 0 && !root.join(parts[..=at].join("/")).exists())
        })
    })
}

/// The first command of `commands` the tool rejects before building, with its first error
/// line. A command that fails for another reason, or whose program cannot start, is the run's
/// to find.
async fn rejected_command(root: &Path, toolbox: &ra_tools::Toolbox, commands: &[String], files: &[String]) -> Option<(String, String)> {
    for command in commands {
        if names_created(command, files) || names_new_package(root, command, files) {
            continue;
        }
        let Some(words) = dry_words(command) else { continue };
        if let Ok((false, output)) = toolbox.check_args(&words).await
            && is_setup_failure(&output)
        {
            return Some((command.clone(), first_error(&output)));
        }
    }
    None
}

/// Dry-runs the acceptance's commands on the base tree, on a worker slot under the project's
/// timeout, and holds the card when the tool rejects one: the worker could change nothing, and
/// asm #131 and #132 ran 272 and 235 times on one malformed argument. The tool's line goes on
/// the feed. `None` sends the card on to its worker.
async fn held_by_preflight(store: &Store, root: &Path, node: &Node, worker: &Worker) -> anyhow::Result<Option<Vec<String>>> {
    if !worker.preflight.unwrap_or(worker.script.is_none()) {
        return Ok(None);
    }
    // a toolbox that cannot be made is the run's to report
    let nodes = store.all().await?;
    let files = ra_roadmap::tree::files_of(&nodes, node).0;
    let Some((command, error)) = preflight(root, node, files, worker.toolbox(root)).await else { return Ok(None) };
    store.set_state_by(node.id, TaskState::Blocked, Source::Run).await?;
    let text = format!("#{} blocked: `{command}` is rejected before any build: {error}; a person fixes the acceptance", node.id);
    tracing::warn!("{text}");
    crate::pair::event(root, &text);
    Ok(Some(vec![format!("  {text}")]))
}

/// The line for a queued card whose state, read again just before its run starts, is one no run
/// starts from: blocked, accepted, cancelled, or implemented with its review pending. `None`
/// sends the card on. A card gone from the roadmap was cancelled, which removes it.
pub async fn skipped_now(store: &Store, id: i64) -> Option<String> {
    let Ok(node) = store.read(id).await else { return Some(format!("skipped: #{id} is cancelled now")) };
    matches!(node.state, TaskState::Blocked | TaskState::Accepted | TaskState::Cancelled | TaskState::Implemented | TaskState::Review)
        .then(|| format!("skipped: #{id} is {} now", node.state))
}

/// Whether the card's last run ended `implemented` and nothing happened since: the card is
/// implemented or in review, the run's handoff is the card's last (a note or a send-back would be
/// a later one), and the tree is at the commit that handoff's head names. Another worker run
/// would only redo it; the review is what the card waits for.
///
/// # Errors
/// When the card cannot be read.
pub async fn reported_implemented(store: &Store, id: i64) -> anyhow::Result<bool> {
    let node = store.read(id).await?;
    if !matches!(node.state, TaskState::Implemented | TaskState::Review) {
        return Ok(false);
    }
    let root = workspace_of(store);
    let task = ra_context::TaskContext::open(&root, id);
    let Some(last) = task.handoffs.last().filter(|h| h.outcome == "implemented") else { return Ok(false) };
    let Some(tip) = ra_context::tip_of(last) else { return Ok(false) };
    let worktree = root.join(WORKTREES_DIR).join(id.to_string());
    let acting = if worktree.join(".git").exists() { worktree } else { root };
    Ok(crate::review::tip_now(&acting).await.as_deref() == Some(tip))
}

/// Whether the work of `id` is already in the base: the card lists `+` files, every one exists in
/// the project, and every command of its acceptance passes there. Card #666 ran four times with
/// its file on main; a card with no `+` file or no named command is never in the base.
///
/// # Errors
/// When the card or the roadmap cannot be read.
pub async fn in_base(store: &Store, id: i64) -> anyhow::Result<bool> {
    let root = workspace_of(store);
    let node = store.read(id).await?;
    let nodes = store.all().await?;
    let files = ra_roadmap::tree::files_of(&nodes, &node).0;
    Ok(is_in_base(&root, &node, files, ra_tools::Toolbox::new(&root)).await)
}

/// [`in_base`] over a node, its files and a toolbox already at hand.
async fn is_in_base(root: &Path, node: &Node, files: &[String], toolbox: anyhow::Result<ra_tools::Toolbox>) -> bool {
    let created: Vec<&str> = files.iter().filter_map(|f| f.trim().strip_prefix('+')).map(str::trim).collect();
    let commands = named_checks(node.acceptance.as_deref());
    if created.is_empty() || commands.is_empty() || !created.iter().all(|f| root.join(f).exists()) {
        return false;
    }
    let Ok(toolbox) = toolbox else { return false };
    let toolbox = toolbox.with_timeout(checks_of(root, node).timeout);
    let _slot = hold_build(root, root).await;
    for command in &commands {
        let Ok(words) = ra_tools::words(command) else { return false };
        if !matches!(toolbox.check_args(&words).await, Ok((true, _))) {
            return false;
        }
    }
    true
}

/// Marks a card whose work is already in the base accepted and starts no worker; `None` sends
/// the card on. A scripted worker goes on, as the preflight's does.
async fn skipped_in_base(store: &Store, root: &Path, node: &Node, worker: &Worker) -> anyhow::Result<Option<Vec<String>>> {
    if worker.script.is_some() {
        return Ok(None);
    }
    let nodes = store.all().await?;
    let files = ra_roadmap::tree::files_of(&nodes, node).0;
    if !is_in_base(root, node, files, worker.toolbox(root)).await {
        return Ok(None);
    }
    store.set_state_by(node.id, TaskState::Accepted, Source::Run).await?;
    let text = format!("#{} already in base: its `+` files exist and its acceptance passes; no worker started", node.id);
    tracing::info!(target: crate::trace::PROGRESS, "{text}");
    crate::pair::event(root, &text);
    Ok(Some(vec![format!("  {text}")]))
}

/// The cards still to run whose acceptance names a command the tool rejects before building,
/// one line each: `rc check` lists them beside the lint, so the acceptance is fixed before a
/// walk reaches the card and holds it.
///
/// # Errors
///
/// When the roadmap cannot be read.
pub async fn preflight_faults(store: &Store) -> anyhow::Result<Vec<String>> {
    let root = workspace_of(store);
    let nodes = store.all().await?;
    let open = ra_roadmap::tree::cards(&nodes)
        .filter(|node| matches!(node.state, TaskState::Defined | TaskState::Ready | TaskState::NeedsRevision));
    let mut faults = Vec::new();
    for node in open {
        let files = ra_roadmap::tree::files_of(&nodes, node).0;
        if let Some((command, error)) = preflight(&root, node, files, ra_tools::Toolbox::new(&root)).await {
            faults.push(format!("#{}: `{command}` is rejected before any build: {error}", node.id));
        }
    }
    Ok(faults)
}

/// The acceptance command of `node` the tool rejects on a dry run in `toolbox`, under the
/// project's timeout and on a worker slot, with its first error line; `None` when the acceptance
/// names no command or the toolbox could not be made.
async fn preflight(root: &Path, node: &Node, files: &[String], toolbox: anyhow::Result<ra_tools::Toolbox>) -> Option<(String, String)> {
    let commands = named_checks(node.acceptance.as_deref());
    if commands.is_empty() {
        return None;
    }
    let toolbox = toolbox.ok()?.with_timeout(checks_of(root, node).timeout);
    let _slot = hold_build(root, root).await;
    rejected_command(root, &toolbox, &commands, files).await
}

/// The programs a worker's shell runs a check through by itself; any other is the
/// orchestrator's unless the project's `[worker] allow` names it.
const WORKER_CHECKS: [&str; 2] = ["cargo", "rustfmt"];

/// What marks a backticked command as long, right after its closing backtick: it outlives a
/// worker's turn, so the orchestrator runs it whatever the worker may run.
pub const LONG_MARK: &str = "(long)";

/// The acceptance commands the orchestrator runs for the worker: the ones the worker may not
/// run (runelite's contract runner), and the ones marked [`LONG_MARK`]. The worker is told not to
/// run them, and their output, not only their exit, reaches the review and the handoff: the
/// reviews of runelite #31, #51, #73 and #152 each asked again for the contract runner's result.
fn orchestrated_checks(acceptance: Option<&str>, worker: &crate::router::WorkerConfig) -> Vec<String> {
    let Some(text) = acceptance else { return Vec::new() };
    let pieces: Vec<&str> = text.split('`').collect();
    let mut found = Vec::new();
    for at in (1..pieces.len()).step_by(2) {
        let span = pieces[at].trim();
        if !is_command(span) {
            continue;
        }
        let long = pieces.get(at + 1).is_some_and(|after| after.trim_start().starts_with(LONG_MARK));
        let program = span.split_whitespace().next().unwrap_or_default();
        let may_run = WORKER_CHECKS.contains(&program) || worker.allows(span);
        if (long || !may_run) && !found.iter().any(|c| c == span) {
            found.push(span.to_owned());
        }
    }
    found
}

/// The line of a worker's prompt that names the commands the orchestrator runs for it, under
/// the project's time limit; empty when there are none.
fn orchestrated_line(checks: &Checks) -> String {
    if checks.orchestrated.is_empty() {
        return String::new();
    }
    let named: Vec<String> = checks.orchestrated.iter().map(|c| format!("`{c}`")).collect();
    format!(
        "The orchestrator runs {} for you after your report, on a worker slot under the project's {} s limit, and attaches the output to the review and the handoff: do not run them yourself. To wait on a long job of your own, use `rc wait <file|pid>`.\n",
        named.join(", "),
        checks.timeout.as_secs()
    )
}

use ra_roadmap::check::is_command;

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
    /// The failures of a failed command, read from its whole output by
    /// [`ra_context::failures_of`]; the next run reads them where the tail would have cut them.
    failures: Vec<String>,
    /// The tests a failed command marks `FAILED`, named as a run of them alone names them;
    /// empty for a passed command and for a failure that names no test.
    tests: Vec<String>,
    /// The first line of a failed command's output that reads as its error, else its first
    /// line; empty for a passed one. The fingerprint and the reason of a block name it.
    first_error: String,
    /// Whether the command failed before any compile or test output: an argument the tool
    /// rejects, a program not found, an SDK location missing. A rerun cannot move such a card.
    setup: bool,
}

/// What a failed command's output says when it never reached a build or a test: the tool
/// rejected its arguments, the program or the SDK is not there.
const SETUP_FAILURES: [&str; 16] = [
    "unexpected argument",
    "unrecognized option",
    "unrecognized argument",
    "unknown option",
    "invalid value",
    "no such command",
    "no such subcommand",
    "did not match any packages",
    "command not found",
    "is not recognized as",
    "no such file or directory",
    "cannot find the path",
    "sdk location not found",
    "could not find `cargo.toml`",
    "no targets specified",
    "not installed",
];

/// The lines of an output that show a build or a test ran: past them a failure is the code's.
const BUILD_OUTPUT: [&str; 10] =
    ["compiling ", "checking ", "running ", "test ", "test result:", "finished ", "> task", "build ", "error[e", "warning:"];

/// The line a failed command's output leads with as its error: the first that names an error,
/// a failed test or a panic, else the first that is not blank.
fn first_error(output: &str) -> String {
    let lines = || output.lines().map(str::trim).filter(|line| !line.is_empty());
    let is_error = |line: &&str| {
        let lower = line.to_lowercase();
        lower.starts_with("error") || lower.contains("error:") || line.ends_with(" FAILED") || lower.contains("panicked")
    };
    lines().find(is_error).or_else(|| lines().next()).unwrap_or_default().chars().take(200).collect()
}

/// Whether a failed command's output is a setup failure: it says so, and no line of it shows
/// a build or a test running.
fn is_setup_failure(output: &str) -> bool {
    let lower = output.to_lowercase();
    let built = lower.lines().map(str::trim_start).any(|line| BUILD_OUTPUT.iter().any(|mark| line.starts_with(mark)));
    !built && SETUP_FAILURES.iter().any(|mark| lower.contains(mark))
}

/// The fingerprint of a failed check: the command, its first error line, the hash of the
/// tree's diff and the commit it is on. A run that ends on the fingerprint its last run ended
/// on failed the same way on the same tree, and a third run would too. FNV-1a, so a fingerprint
/// kept in a context file reads the same to the next build of the agent.
fn check_fingerprint(command: &str, first_error: &str, diff: &str, base: &str) -> String {
    let hash = |text: &str| {
        text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3))
    };
    let diff = format!("{:016x}", hash(diff));
    format!("{:016x}", hash(&format!("{command}\n{first_error}\n{diff}\n{}", base.trim())))
}

/// The output of a read-only git command in `root`, or nothing where git has nothing to say.
async fn git_text(root: &Path, args: &[&str]) -> String {
    match tokio::process::Command::new("git").args(args).current_dir(root).output().await {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).into_owned(),
        _ => String::new(),
    }
}

/// The fingerprint of a reviewer's points: each trimmed and lowercased, in sorted order, so the
/// same points in another order or case read the same. FNV-1a, as [`check_fingerprint`].
fn points_fingerprint(points: &[String]) -> String {
    let mut points: Vec<String> = points.iter().map(|point| point.trim().to_lowercase()).collect();
    points.sort();
    let hash = points
        .join("\n")
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3));
    format!("{hash:016x}")
}

/// The phrases of a report that hands its card to a person: what is left only a person can give.
const PERSON_PHRASES: [&str; 6] =
    ["only a person", "only the person", "needs a person", "needs the person", "a person must", "a person has to"];

/// The first line of a report that says only a person can move the card, clipped to 200
/// characters; `None` when the report asks nothing of a person.
fn person_line(report: &str) -> Option<String> {
    let line = report.lines().map(str::trim).find(|line| {
        let lower = line.to_lowercase();
        PERSON_PHRASES.iter().any(|phrase| lower.contains(phrase))
    })?;
    Some(line.trim_start_matches(['-', '*', '#', '>', ' ']).chars().take(200).collect())
}

/// What a `[machine]` note says when a tool is gone from the machine.
const MISSING_TOOL_PHRASES: [&str; 5] = ["not installed", "isn't installed", "is missing", "not on this machine", "no such tool"];

/// The first `[machine]` line of a report that says an external tool is missing or not installed
/// (csmith in WSL, a prebuilt binary), clipped to 200 characters: only a person can install it,
/// so a rerun would meet the same gap. `None` when no machine note names a missing tool.
pub fn missing_tool_line(report: &str) -> Option<String> {
    let line = report.lines().map(str::trim).find(|line| {
        let lower = line.to_lowercase();
        lower.contains("[machine]") && MISSING_TOOL_PHRASES.iter().any(|phrase| lower.contains(phrase))
    })?;
    Some(line.trim_start_matches(['-', '*', '#', '>', ' ']).chars().take(200).collect())
}

/// The reason a worker's report gives under its `## Blocked` heading: the rest of the heading's
/// line, else the first line under it. `None` when the report has no such heading.
fn blocked_reason(report: &str) -> Option<String> {
    let mut lines = report.lines().map(str::trim);
    let heading = lines
        .find(|line| line.strip_prefix("## Blocked").is_some_and(|rest| rest.is_empty() || !rest.starts_with(char::is_alphanumeric)))?;
    let own = heading.trim_start_matches("## Blocked").trim_start_matches([':', ' ', '-', '—', '·']).trim();
    let reason = if own.is_empty() { lines.find(|line| !line.is_empty()).unwrap_or("the worker reported the card blocked") } else { own };
    Some(reason.chars().take(200).collect())
}

/// Why a run holds its card for a person at once instead of sending it back: a report under
/// `## Blocked` from a run that changed nothing, a check that failed before any build, or one
/// that failed as the last run's did. `None` leaves the verdict to the check.
fn held_reason(report: &str, changed: &[String], failed: Option<&CheckRun>, print: Option<&str>, last: Option<&str>) -> Option<String> {
    if changed.is_empty()
        && let Some(reason) = blocked_reason(report)
    {
        return Some(format!("the worker reported it blocked: {reason}"));
    }
    if changed.is_empty()
        && let Some(line) = missing_tool_line(report)
    {
        return Some(format!("a tool is missing, only a person can install it: {line}"));
    }
    let failed = failed?;
    if failed.setup {
        return Some(format!("the check failed before any build: {}: {}", failed.command, failed.first_error));
    }
    (print.is_some() && print == last)
        .then(|| format!("the check failed as the last run's did: {}: {}", failed.command, failed.first_error))
}

/// How many lines of a passed command's output the notes keep.
const PASSED_TAIL_LINES: usize = 5;

/// The output a passed, filtered `cargo test` keeps: the result lines of the binaries that ran
/// a test, so the last binary's `0 passed; 11 filtered out` cannot read as the test not
/// running. Every binary reporting `0 passed` is a filter that matched no test: an error. An
/// output with no result line at all (not a cargo test's) is kept as it is.
///
/// # Errors
/// When the output has result lines and none of them ran a test.
pub fn ran_of(output: &str) -> anyhow::Result<String> {
    let results: Vec<&str> = output.lines().filter(|line| line.trim_start().starts_with("test result:")).collect();
    if results.is_empty() {
        return Ok(output.to_owned());
    }
    let ran: Vec<&str> = results
        .iter()
        .copied()
        .filter(|line| {
            let before = line.split(" passed").next().unwrap_or_default();
            before.rsplit(' ').next().and_then(|n| n.parse::<u64>().ok()).is_some_and(|n| n > 0) && line.contains(" passed")
        })
        .collect();
    if ran.is_empty() {
        anyhow::bail!("the filter matched no test: every test binary reported 0 passed\n{}", results.join("\n"));
    }
    Ok(ran.join("\n"))
}

/// Runs the checks of a card in the project directory, in the order written, and stops at the
/// first failure; each command's verdict goes to the activity log as it comes. Each command is
/// split into its words once, here, and they reach the program as its arguments. A passed
/// command among `orchestrated` keeps the tail a failed one would: the worker never saw its
/// output, and the reviewer judges by it.
async fn run_checks(toolbox: &anyhow::Result<ra_tools::Toolbox>, commands: &[String], orchestrated: &[String]) -> Vec<CheckRun> {
    let mut runs = Vec::new();
    for command in commands {
        let outcome = match (toolbox, ra_tools::words(command)) {
            (Ok(toolbox), Ok(words)) => toolbox.check_args(&words).await,
            (Err(e), _) => Err(anyhow::anyhow!("{e:#}")),
            (_, Err(e)) => Err(e),
        };
        let outcome = match (outcome, toolbox, ra_tools::words(command)) {
            (Ok((false, output)), Ok(toolbox), Ok(words)) => Ok(rerun_alone(toolbox, &words, output).await),
            (outcome, ..) => outcome,
        };
        let outcome = match outcome {
            Ok((true, output)) if names_tests(command) && command.contains("cargo") => ran_of(&output).map(|kept| (true, kept)),
            outcome => outcome,
        };
        let kept = if orchestrated.contains(command) { TAIL_LINES } else { PASSED_TAIL_LINES };
        let run = match outcome {
            Ok((true, output)) => CheckRun {
                command: command.clone(),
                passed: true,
                exit: output.lines().last().unwrap_or_default().to_owned(),
                text: last_lines(&output, kept),
                failed_tests: 0,
                failures: Vec::new(),
                tests: Vec::new(),
                first_error: String::new(),
                setup: false,
            },
            Ok((false, output)) => {
                let failure = check_failure(&output);
                let gradle = command.split_whitespace().next().is_none_or(|program| !program.to_lowercase().contains("cargo"));
                CheckRun {
                    command: command.clone(),
                    passed: false,
                    exit: output.lines().last().unwrap_or_default().to_owned(),
                    text: failure.text,
                    failed_tests: failure.failed,
                    failures: ra_context::failures_of(&output),
                    tests: failed_test_names(&output, gradle),
                    first_error: first_error(&output),
                    setup: is_setup_failure(&output),
                }
            }
            Err(e) => {
                let text = format!("{e:#}");
                CheckRun {
                    command: command.clone(),
                    passed: false,
                    exit: text.clone(),
                    first_error: first_error(&text),
                    setup: is_setup_failure(&text),
                    text,
                    failed_tests: 0,
                    failures: Vec::new(),
                    tests: Vec::new(),
                }
            }
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

/// The runs of its card's own check a worker may ask for in one run, `rc check <id> --mid`: in
/// a project whose builds are refused a hard card was written blind, and each attempt was a
/// whole run judged by a check the worker never saw (asm #233, #265 and #269 of 2026-11-14/15,
/// three or four runs each, about 7 dollars a card).
pub const MID_CHECKS_A_RUN: usize = 2;

/// The file the mid-run checks of the current run of `card` in `tree` are noted in, one line
/// each: `var/mid-checks/<card>` of the tree, beside its build count.
fn mid_checks_file(tree: &Path, card: i64) -> PathBuf {
    tree.join("var").join("mid-checks").join(card.to_string())
}

/// The lines of the mid-run checks the current run of `card` in `tree` has run, in order, for
/// its handoff.
#[must_use]
pub fn mid_check_lines(tree: &Path, card: i64) -> Vec<String> {
    let text = std::fs::read_to_string(mid_checks_file(tree, card)).unwrap_or_default();
    text.lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_owned).collect()
}

/// Gives the run of `card` in `tree` its [`MID_CHECKS_A_RUN`] afresh, at the run's start.
pub fn reset_mid_checks(tree: &Path, card: i64) {
    let _ = std::fs::remove_file(mid_checks_file(tree, card));
}

/// What a mid-run check came to: whether every command passed, and what the worker is shown.
#[derive(Debug, Clone)]
pub struct MidCheck {
    pub passed: bool,
    /// One `passed`/`failed` line a command up to the first failure, then that failure's first
    /// failed test and its tail, as the notes clip a failed check.
    pub text: String,
}

/// Runs the checks of `node` in `tree` for its worker, mid-run: the commands the orchestrator's
/// check runs at the end, under the project's check timeout, on one of the project's build
/// slots and in the tree's own cargo target. The use is noted in the tree, for the run's
/// handoff, and on the project's activity log; the orchestrator's own check is unchanged.
///
/// # Errors
/// When the run has spent its [`MID_CHECKS_A_RUN`], or the tree cannot be noted in.
pub async fn mid_run_check(tree: &Path, node: &Node) -> anyhow::Result<MidCheck> {
    let id = node.id;
    let spent = mid_check_lines(tree, id).len();
    if spent >= MID_CHECKS_A_RUN {
        anyhow::bail!(
            "this run has spent its {MID_CHECKS_A_RUN} mid-run checks of #{id}: report the change as it stands; the orchestrator's check runs after the report, and a failure comes back as a rerun with its output"
        );
    }
    // the tree's path as given, not canonical: a verbatim `\\?\` root takes no `var/…` join
    let (root, _) = ra_providers::claude_code::project_of(tree);
    let Checks { commands, timeout, .. } = checks_of(&root, node);
    let own_target = ra_providers::claude_code::shared_target(tree);
    let scratch = ra_providers::claude_code::scratch_env(tree).into_iter().map(|(key, dir)| (key, dir.display().to_string()));
    let target_env = own_target.iter().map(|t| ("CARGO_TARGET_DIR".to_owned(), t.display().to_string()));
    let toolbox = ra_tools::Toolbox::new(tree).map(|toolbox| toolbox.with_timeout(timeout).with_env(scratch.chain(target_env)));
    // spent when it starts: a check the worker's shell gave up on still held a slot
    let file = mid_checks_file(tree, id);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let started = format!("mid-run check {} of {MID_CHECKS_A_RUN}: {}", spent + 1, commands.join(" · then "));
    let mut lines = mid_check_lines(tree, id);
    lines.push(format!("{started} · running"));
    std::fs::write(&file, format!("{}\n", lines.join("\n")))?;
    let _slot = hold_build(&root, tree).await;
    let target = own_target.unwrap_or_else(|| crate::hub::cargo_target(&root));
    let cargo = commands.iter().any(|c| c.trim_start().starts_with("cargo"));
    let waited = if cargo { guard_target(&root, tree, &target, timeout).await } else { Ok(None) };
    let (passed, text) = match waited {
        Ok(_guard) => {
            let runs = run_checks(&toolbox, &commands, &[]).await;
            let passed = runs.iter().all(|run| run.passed);
            let mut text: Vec<String> =
                runs.iter().map(|run| format!("{} {}", if run.passed { "passed" } else { "failed" }, run.command)).collect();
            if let Some(failed) = runs.iter().find(|run| !run.passed) {
                text.extend([String::new(), failed.text.clone()]);
            }
            (passed, text.join("\n"))
        }
        Err(text) => (false, format!("failed {}\n\n{text}", commands.join(" · then "))),
    };
    let verdict = if passed { "passed" } else { "failed" };
    lines.pop();
    lines.push(format!("{started} · {verdict}"));
    std::fs::write(&file, format!("{}\n", lines.join("\n")))?;
    note_activity(&root, &crate::logs::tag_card(id, &format!("  {started} · {verdict}")));
    Ok(MidCheck { passed, text })
}

/// Appends `line` to the activity log of the project at `root`, which a running window tails.
fn note_activity(root: &Path, line: &str) {
    use std::io::Write as _;
    let file = root.join(crate::ui::ACTIVITY_FILE);
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let written = std::fs::OpenOptions::new().create(true).append(true).open(&file).and_then(|mut out| writeln!(out, "{line}"));
    if let Err(e) = written {
        tracing::warn!("the activity line was not written: {e}");
    }
}

/// The failures the project's own tree gives for `command`: the base a card's worktree started
/// from. None when the command passes there or cannot run.
async fn base_failures(worker: &Worker, root: &Path, command: &str, timeout: std::time::Duration) -> Vec<String> {
    let Ok(toolbox) = worker.toolbox(root).map(|toolbox| toolbox.with_timeout(timeout)) else { return Vec::new() };
    // a wait on the project's target that ran out leaves the failures unmarked, not the check failed
    let _target = if command.trim_start().starts_with("cargo") {
        match guard_target(root, root, &crate::hub::cargo_target(root), timeout).await {
            Ok(guard) => guard,
            Err(_) => return Vec::new(),
        }
    } else {
        None
    };
    match toolbox.check_acceptance(command).await {
        Ok((false, output)) => ra_context::failures_of(&output),
        _ => Vec::new(),
    }
}

/// The runs of failed tests on a base commit, one line each: the commit, the tests asked about
/// and the ones that failed there, tab-separated, each set of names space-separated.
const BASE_RUNS_FILE: &str = "var/runs/base-tests.log";

/// The cards that wait on a red main, one line each: the card and the commit it failed on,
/// tab-separated.
const RED_MAIN_FILE: &str = "var/runs/red-main";

/// What a card that waits on main is told will move it.
const RED_MAIN_REQUEUE: &str = "the card goes back on the queue when main's commit changes";

/// What the project's base commit made of a card's failed tests.
struct BaseRun {
    /// The commit the tests ran on.
    sha: String,
    /// The card's tests that failed there too.
    failing: Vec<String>,
}

/// The lock of each project's base runs: two cards failing at once ask about the same commit,
/// and the second reads the first's answer rather than build it again.
type BaseLocks = Mutex<std::collections::HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>;
static BASE_LOCKS: std::sync::LazyLock<BaseLocks> = std::sync::LazyLock::new(BaseLocks::default);

/// Which of `tests`, failed in a card's check `command`, fail on the project's base commit too:
/// they run alone there, in a scratch worktree on the check's build slot, once per commit and
/// test set, the answer kept by the commit's sha. The run that finds main red says so on the
/// feed, once. `None` when the base cannot be read, the tests cannot run alone, or the base
/// failed without naming a test.
async fn main_failures(worker: &Worker, root: &Path, command: &str, tests: &[String], timeout: std::time::Duration) -> Option<BaseRun> {
    let sha = git(root, &["rev-parse", "HEAD"]).await.ok()?;
    let words = ra_tools::words(command).ok()?;
    let mut asked = tests.to_vec();
    asked.sort();
    asked.dedup();
    let rerun = rerun_words(&words, &asked)?;
    let lock = BASE_LOCKS.lock().entry(root.to_path_buf()).or_default().clone();
    let _held = lock.lock().await;
    let key = asked.join(" ");
    if let Some(failing) = cached_base_run(root, &sha, &key) {
        return Some(BaseRun { sha, failing });
    }
    let path = match base_worktree(root, &sha).await {
        Ok(path) => path,
        Err(e) => {
            tracing::warn!("the failed tests were not run on the base {}: {e:#}", short_sha(&sha));
            return None;
        }
    };
    let own_target = ra_providers::claude_code::shared_target(&path);
    let toolbox = worker
        .toolbox(&path)
        .ok()?
        .with_timeout(timeout)
        .with_env(own_target.iter().map(|t| ("CARGO_TARGET_DIR".to_owned(), t.display().to_string())));
    // the build slot is the card's check's, still held, and the base worktree builds in its own
    // target, `wt-base`, which cargo's own lock guards: the hub's lock taken again in this process
    // waited out the check's whole timeout (every red-main test hung for twenty minutes)
    let (passed, output) = toolbox.check_args(&rerun).await.ok()?;
    let gradle = !words[0].to_lowercase().contains("cargo");
    let failing: Vec<String> =
        if passed { Vec::new() } else { failed_test_names(&output, gradle).into_iter().filter(|name| asked.contains(name)).collect() };
    // a base that fails without naming a test (a compile error, a timeout) says nothing of them
    if !passed && failing.is_empty() {
        return None;
    }
    keep_base_run(root, &sha, &key, &failing);
    if !failing.is_empty() {
        let text = format!("main is red: {} at {}", failing.join(", "), short_sha(&sha));
        tracing::warn!("{text}");
        crate::pair::event(root, &text);
    }
    Some(BaseRun { sha, failing })
}

/// The tests of `tests` that failed on `sha` when a base run asked about exactly them.
fn cached_base_run(root: &Path, sha: &str, tests: &str) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(root.join(BASE_RUNS_FILE)).ok()?;
    text.lines().rev().find_map(|line| {
        let mut fields = line.split('\t');
        (fields.next() == Some(sha) && fields.next() == Some(tests))
            .then(|| fields.next().unwrap_or_default().split_whitespace().map(str::to_owned).collect())
    })
}

/// Keeps a base run's answer for the next card that asks about the same commit and tests.
fn keep_base_run(root: &Path, sha: &str, tests: &str, failing: &[String]) {
    use std::io::Write as _;
    let file = root.join(BASE_RUNS_FILE);
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file)
        .and_then(|mut out| writeln!(out, "{sha}\t{tests}\t{}", failing.join(" ")));
    if let Err(e) = written {
        tracing::warn!("the base run on {} was not kept: {e}", short_sha(sha));
    }
}

/// Notes card `id` as waiting on main at `sha`, in place of any earlier line of it.
fn wait_on_main(root: &Path, id: i64, sha: &str) {
    let file = root.join(RED_MAIN_FILE);
    let own = format!("{id}\t");
    let mut lines: Vec<String> =
        std::fs::read_to_string(&file).unwrap_or_default().lines().filter(|line| !line.starts_with(&own)).map(str::to_owned).collect();
    lines.push(format!("{id}\t{sha}"));
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&file, format!("{}\n", lines.join("\n"))) {
        tracing::warn!("#{id} was not noted as waiting on main: {e}");
    }
}

/// Puts the cards that wait on a red main back on the queue once the project's commit is not
/// the one they failed on: main moved, and their tests may pass on it now. The reason leaves
/// the head of their notes; a card a person moved meanwhile only leaves the list. One line per
/// card put back, on the feed too.
pub async fn requeue_red_main(store: &Store, root: &Path) -> Vec<String> {
    let file = root.join(RED_MAIN_FILE);
    let Ok(text) = std::fs::read_to_string(&file) else { return Vec::new() };
    let Ok(head) = git(root, &["rev-parse", "HEAD"]).await else { return Vec::new() };
    let mut kept = Vec::new();
    let mut lines = Vec::new();
    for line in text.lines() {
        let Some((id, sha)) = line.split_once('\t') else { continue };
        let Ok(id) = id.parse::<i64>() else { continue };
        let Ok(node) = store.read(id).await else { continue };
        if node.state != TaskState::Blocked {
            continue;
        }
        if sha == head || store.set_state_by(id, TaskState::Defined, Source::Run).await.is_err() {
            kept.push(line.to_owned());
            continue;
        }
        if let Some(notes) = node.notes.as_deref()
            && notes.starts_with("main fails ")
        {
            let rest = notes.split_once("\n\n").map_or("", |(_, rest)| rest);
            let _ = store.set_notes(id, rest).await;
        }
        let text = format!("#{id} back on the queue: main moved from {} to {}", short_sha(sha), short_sha(&head));
        tracing::info!(target: crate::trace::PROGRESS, "{text}");
        crate::pair::event(root, &text);
        lines.push(text);
    }
    let written = if kept.is_empty() { std::fs::remove_file(&file) } else { std::fs::write(&file, format!("{}\n", kept.join("\n"))) };
    if let Err(e) = written {
        tracing::warn!("the cards waiting on main were not rewritten: {e}");
    }
    lines
}

/// The first seven characters of a commit's sha, as git abbreviates it.
fn short_sha(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

/// The name a run of a test alone takes for a failure's name line: gradle's `Class > method()`
/// as `Class.method`, any other line as it is.
fn run_name(head: &str) -> String {
    match head.split_once(" > ") {
        Some((class, method)) => format!("{class}.{}", method.trim_end_matches("()")),
        None => head.to_owned(),
    }
}

/// The names of the tests a failed cargo or gradle output marks `FAILED`: `test name ... FAILED`,
/// `name --- FAILED` under `-q`, or gradle's `Class > method FAILED` as `Class.method`.
fn failed_test_names(output: &str, gradle: bool) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for line in output.lines().map(str::trim).filter(|line| line.ends_with(" FAILED")) {
        let name = if gradle {
            line.strip_suffix(" FAILED")
                .and_then(|head| head.split_once(" > "))
                .map(|(class, method)| format!("{class}.{}", method.trim_end_matches("()")))
        } else if let Some(rest) = line.strip_prefix("test ") {
            rest.strip_suffix(" ... FAILED").map(str::to_owned)
        } else {
            line.split_once(" --- ").map(|(name, _)| name.to_owned())
        };
        if let Some(name) = name
            && !name.is_empty()
            && !names.contains(&name)
        {
            names.push(name);
        }
    }
    names
}

/// The command that runs only `names`: the cargo test's own flags with each name matched
/// exactly, or the gradle wrapper with a `--tests` per name. `None` for any other command.
fn rerun_words(words: &[String], names: &[String]) -> Option<Vec<String>> {
    let program = words.first()?.rsplit(['/', '\\']).next()?.to_lowercase();
    // a compile ([`compile_of`]) runs no test: a failure in it has none to run alone
    if names.is_empty() || words.iter().any(|word| word == "--no-run" || word.ends_with("testClasses")) {
        return None;
    }
    if program.starts_with("cargo") && words.get(1).is_some_and(|word| word == "test") {
        const VALUED: [&str; 8] = ["-p", "--package", "--manifest-path", "--features", "-F", "--exclude", "--test", "--target"];
        let mut rerun = vec![words[0].clone(), "test".to_owned()];
        let mut rest = words[2..].iter().take_while(|word| *word != "--");
        while let Some(word) = rest.next() {
            if word.starts_with('-') {
                rerun.push(word.clone());
                if VALUED.contains(&word.as_str())
                    && let Some(value) = rest.next()
                {
                    rerun.push(value.clone());
                }
            }
        }
        rerun.extend(["--".to_owned(), "--exact".to_owned()]);
        rerun.extend(names.iter().cloned());
        return Some(rerun);
    }
    if program.contains("gradlew") || program.starts_with("gw") || program.starts_with("gradle") {
        let mut rerun = words.to_vec();
        for name in names {
            rerun.extend(["--tests".to_owned(), name.clone()]);
        }
        return Some(rerun);
    }
    None
}

/// Gives a failed test check a second chance alone: load makes a timing test fail in a full run
/// and pass by itself. Tests that all pass alone make the check pass, the output naming them
/// as load-sensitive; any that fails again keeps the check failed with that rerun's output.
async fn rerun_alone(toolbox: &ra_tools::Toolbox, words: &[String], output: String) -> (bool, String) {
    let gradle = !words.first().is_some_and(|word| word.to_lowercase().contains("cargo"));
    let names = failed_test_names(&output, gradle);
    let Some(rerun) = rerun_words(words, &names) else {
        return (false, output);
    };
    match toolbox.check_args(&rerun).await {
        Ok((true, again)) => {
            let note = format!("load-sensitive, passed alone: {}", names.join(", "));
            tracing::info!(target: crate::trace::PROGRESS, "{note}");
            (true, format!("{note}\n{again}"))
        }
        Ok((false, again)) => (false, again),
        Err(_) => (false, output),
    }
}

/// The last `n` lines of a text.
fn last_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// How many lines of a failed check's output the notes keep after the first failure.
const TAIL_LINES: usize = 60;

/// How long a card's check, or its lint, may run unless the project's `check_timeout` says
/// otherwise: a full workspace test run outlives a session here (notes.log), and the walk
/// must not hang on it.
pub const CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_mins(20);

/// What a failed check's output comes to for the notes.
struct CheckFailure {
    /// The tests cargo marked `FAILED`; zero where the output is not a test run's.
    failed: usize,
    /// The first `test … FAILED` line, then the last [`TAIL_LINES`] lines.
    text: String,
}

/// Reads a failed check's output: the first failed test leads, the tail follows, so a reader
/// (the next run, the person) sees what broke before the noise around it, and a long run's
/// output does not carry whole into the next turn. A failed test is `test name ... FAILED`, or
/// `name --- FAILED` under `-q`; cargo names it again in its `failures:` list, and each name
/// counts once.
fn check_failure(output: &str) -> CheckFailure {
    let mut seen: Vec<&str> = Vec::new();
    for line in output.lines().map(str::trim) {
        let is_failure = line.ends_with(" FAILED") && (line.starts_with("test ") || line.contains(" --- "));
        if is_failure && !seen.contains(&line) {
            seen.push(line);
        }
    }
    let tail = last_lines(output, TAIL_LINES);
    let text = match seen.first() {
        Some(first) => format!("{first}\n\n{tail}"),
        None => tail,
    };
    CheckFailure { failed: seen.len(), text }
}

/// The words a worker's report uses for a check it started and did not wait for.
const CHECK_CLAIMS: [&str; 5] = ["running", "background", "pending", "not finished", "not yet returned"];

/// A worker's report without the lines that say its check is still running: a check the
/// session outlived in a background shell is not a result, and the agent's own run of the
/// checks takes the lines' place. `None` when the report makes no such claim.
fn without_check_claims(report: &str) -> Option<String> {
    let is_claim = |line: &str| {
        let lower = line.to_lowercase();
        (lower.contains("check") || lower.contains("cargo test")) && CHECK_CLAIMS.iter().any(|word| lower.contains(word))
    };
    if !report.lines().any(is_claim) {
        return None;
    }
    let kept: Vec<&str> = report.lines().filter(|line| !is_claim(line)).collect();
    Some(kept.join("\n").trim().to_owned())
}

/// The turns a second ask for the report gets: the work is done and in the session, a `git
/// diff` at most stands between the session and its report.
const REPORT_TURNS: u32 = 3;

/// The lines a check prints that a worker pastes in place of a report: cargo's build and test
/// lines, a compiler note, a gradle task line.
const CHECK_OUTPUT: [&str; 12] =
    ["running ", "test ", "test result:", "Compiling", "Finished", "Running", "Doc-tests", "warning", "error", "-->", "BUILD ", "> Task"];

/// Whether a worker's report hands nothing off: it names no file in backticks, and what is
/// left once the check's own output and the words for a check still running are taken out is
/// under a sentence (four words). A reviewer would judge the placeholder and send the card back
/// for it, so the run asks the session for the report once more instead.
pub(crate) fn is_empty_handoff(report: &str) -> bool {
    let is_path = |span: &str| {
        !span.is_empty()
            && !span.contains(char::is_whitespace)
            && (span.contains('/')
                || span.contains('\\')
                || span
                    .rsplit_once('.')
                    .is_some_and(|(stem, ext)| !stem.is_empty() && !ext.is_empty() && ext.chars().all(char::is_alphanumeric)))
    };
    if report.split('`').skip(1).step_by(2).map(str::trim).any(is_path) {
        return false;
    }
    // a line that says the suite, the build or the tests are still running says nothing either
    let is_claim = |line: &str| {
        let lower = line.to_lowercase();
        ["check", "cargo", "test", "suite", "build"].iter().any(|word| lower.contains(word))
            && CHECK_CLAIMS.iter().any(|word| lower.contains(word))
    };
    let words: usize = report
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty() && !line.starts_with('#') && !is_claim(line) && !CHECK_OUTPUT.iter().any(|lead| line.starts_with(lead))
        })
        .map(|line| line.split_whitespace().count())
        .sum();
    words < 4
}

/// The prompt of the second ask: the report the first answer was not, and the files git status
/// shows the run changed, so the session reports on those rather than on the check.
fn report_ask(changed: &[String]) -> String {
    let files = if changed.is_empty() {
        "git status shows no file changed by this run".to_owned()
    } else {
        format!("The files git status shows changed by this run: {}", changed.join(", "))
    };
    format!(
        "Your answer named no file and no result, and the check is the orchestrator's to run, not yours. Give the report now, in a few lines: what you changed, naming the file each change landed in in backticks, and what you verified; do not run the check again.\n{files}"
    )
}

/// The runs that end in fewer turns than this are too short to have got anywhere: the next run
/// is told why the last one stopped.
const SHORT_RUN_TURNS: u64 = 3;

/// The note a card's next handoff carries when its last run ended unfinished (it continues, or
/// failed) in under [`SHORT_RUN_TURNS`] turns: the turns and the stop reason the run recorded in
/// its check line, or `no reason recorded`. `None` for a longer run or one that finished.
#[must_use]
pub fn short_run_note(last: &ra_context::Handoff, turns: u64) -> Option<String> {
    let unfinished = last.outcome == ra_context::CONTINUES || last.outcome == "failed";
    if !unfinished || turns >= SHORT_RUN_TURNS {
        return None;
    }
    let reason = last.check.trim().strip_prefix("no check:").map(str::trim).filter(|why| !why.is_empty()).unwrap_or("no reason recorded");
    Some(format!("The last run of this card ended unfinished after {turns} turn(s): {reason}"))
}

/// The note a continuing run's handoff carries: the files the card's tree adds or changes against
/// `main`, the committed, the modified and the untracked ones, each with its line counts, so the
/// session writes on rather than finds them again. `None` when the tree differs from `main` in
/// nothing or git cannot say.
pub async fn prior_files_note(root: &Path, main: &str) -> Option<String> {
    async fn git(root: &Path, args: &[&str]) -> Option<String> {
        let out = tokio::process::Command::new("git").args(args).current_dir(root).output().await.ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }
    let base = git(root, &["merge-base", "HEAD", main]).await?;
    let mut rows = Vec::new();
    for line in git(root, &["diff", "--numstat", base.trim()]).await?.lines() {
        let mut cols = line.splitn(3, '\t');
        if let (Some(added), Some(removed), Some(path)) = (cols.next(), cols.next(), cols.next()) {
            rows.push(format!("- `{path}` +{added} -{removed}"));
        }
    }
    for path in git(root, &["ls-files", "--others", "--exclude-standard"]).await?.lines() {
        let lines = std::fs::read_to_string(root.join(path)).map_or(0, |text| text.lines().count());
        rows.push(format!("- `{path}` new, {lines} line(s)"));
    }
    if rows.is_empty() {
        return None;
    }
    Some(format!(
        "Earlier runs of this card already added or changed these files against {main}; continue in them, do not look for them again:\n{}",
        rows.join("\n")
    ))
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

/// The build rules every run carries, stated once here: a worker that learns them from a refused
/// or timed-out command has already spent the turns.
const BUILD_RULES: &str = "machine:\n\
- a first `cargo build` or `cargo test` compile runs past the 120 s foreground limit: start it with `run_in_background` on the first call and wait on its task notification\n\
- a first compile behind other projects can exceed nine minutes; `rc wait <id>` gives up at nine\n\
- write tool outputs under `var/`, never `%TEMP%`";

/// The machine notes of a run: what the gotchas say about the last hits in the journal, for the
/// system prompt, so the worker does not find out the hard way again, less what `project`, the
/// context the prompt already carries, or an earlier note says. Empty when nothing fired. `card`
/// is the id and the acceptance of the card the run works, which a look-back is judged against.
fn machine_notes(workspace: &Path, project: &str, card: (i64, Option<&str>)) -> String {
    // the journal is the project's, which every worktree's runs write to
    let (home, _) = ra_providers::claude_code::project_of(workspace);
    let gotchas = match Catalogue::load(&home.join(GOTCHAS_FILE)) {
        Ok(catalogue) => catalogue.notes(&Journal::at(&home.join(GOTCHAS_LOG)).recent(RECENT_HITS)),
        Err(_) => String::new(),
    };
    // what the earlier runs said themselves, in debug mode, rides along
    let retro = crate::retro::machine_lines_for(workspace, Some(card));
    let mut checks = check_commands(card.1);
    if let Some(check) = crate::router::Router::load(workspace).ok().and_then(|router| router.worker.check().map(str::to_owned)) {
        checks.push(check);
    }
    let retro = machine_notes_keep(&retro, workspace, &checks);
    let notes = [gotchas, retro].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n\n");
    ra_context::dedupe_notes(&notes, project)
}

/// The heading of the block of what other projects learned, in [`crate::retro::machine_lines_for`].
const ELSEWHERE: &str = "What runs in other projects";

/// `notes` with the other projects' lines cut to those that name something this project has: a
/// line naming a path or a command keeps when a path exists under `root` or the command's program
/// is one of `checks`'; a line naming neither is a general lesson and keeps. A line whose words
/// another kept line holds, bar one, is a repeat and goes. This project's own lines are left.
fn machine_notes_keep(notes: &str, root: &Path, checks: &[String]) -> String {
    let programs: Vec<&str> = checks.iter().filter_map(|check| check.split_whitespace().next()).collect();
    let words = |line: &str| -> std::collections::BTreeSet<String> {
        let text = line.split_once(") ").map_or(line, |(_, rest)| rest);
        text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
    };
    let mut kept_words: Vec<std::collections::BTreeSet<String>> = Vec::new();
    let mut blocks: Vec<String> = Vec::new();
    for block in notes.split("\n\n") {
        if !block.starts_with(ELSEWHERE) {
            blocks.push(block.to_owned());
            continue;
        }
        let mut lines: Vec<&str> = Vec::new();
        for line in block.lines() {
            let Some(text) = line.strip_prefix("- ") else {
                lines.push(line);
                continue;
            };
            let mut named = false;
            let mut here = false;
            for span in text.split('`').skip(1).step_by(2).map(str::trim) {
                if is_command(span) {
                    named = true;
                    here |= span.split_whitespace().next().is_some_and(|program| programs.contains(&program));
                }
            }
            for word in text.split(|c: char| c.is_whitespace() || matches!(c, '`' | '(' | ')' | ',' | ';' | '\'' | '"')) {
                let path = word.trim_end_matches(['.', ':']);
                if path.contains('/') && !path.contains("://") && path.len() > 1 {
                    named = true;
                    here |= root.join(path.trim_start_matches("./")).exists();
                }
            }
            if named && !here {
                continue;
            }
            let set = words(text);
            if set.len() >= 4 && kept_words.iter().any(|kept| set.difference(kept).count() <= 1) {
                continue;
            }
            kept_words.push(set);
            lines.push(line);
        }
        if lines.len() > 1 {
            blocks.push(lines.join("\n"));
        }
    }
    blocks.into_iter().filter(|b| !b.is_empty()).collect::<Vec<_>>().join("\n\n")
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
    /// The project's cap that ended the run, when one did: the card continues in a fresh
    /// session instead of being judged.
    capped: Option<Capped>,
    /// The files the run read, one entry per read, relative to the workspace.
    reads: Vec<String>,
    /// The tool calls whose answers were the largest, biggest first, with their sizes in
    /// characters, when the loop ran the tools itself: a harness keeps its tool results out of
    /// sight.
    biggest: Vec<(String, usize)>,
    /// The read-in: the input tokens of the turns through the first edit, as
    /// [`ra_providers::Completion::read_in`] has it.
    read_in: u64,
    /// How many distinct files the run read whole.
    whole_reads: u64,
}

/// The tokens of a session for the end line: the input with its cached share, then the read-in
/// and the files read whole, as `in 2900000 (91% cached) out 1200 · read-in 740k, 9 files whole`.
fn tokens_line(session: &Session) -> String {
    let usage = &session.usage;
    let cached = usage.cached_tokens * 100 / usage.prompt_tokens.max(1);
    format!(
        "in {} ({cached}% cached) out {} · read-in {}, {} files whole",
        usage.prompt_tokens,
        usage.completion_tokens,
        kilo(session.read_in),
        session.whole_reads
    )
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
/// acceptance since no check ran: its `open` checks by name, and the card's files no run has
/// touched yet (`untouched`).
fn capped_report(cap: Capped, session: &Session, changed: &[String], node: &Node, open: &[String], untouched: &[String]) -> String {
    let mut text = format!(
        "{cap} ended the run after {} turn(s), {} tool call(s); the card continues in a fresh session\n",
        session.turns, session.calls
    );
    if changed.is_empty() {
        text.push_str("changed: nothing\n");
    } else {
        let _ = writeln!(text, "changed: {}", changed.join(", "));
    }
    let acceptance = node.acceptance.as_deref().map(str::trim).filter(|a| !a.is_empty()).unwrap_or("none written");
    let _ = writeln!(text, "left: the check did not run; the acceptance stands: {acceptance}");
    if !open.is_empty() {
        let _ = writeln!(text, "left to pass: {}", open.iter().map(|item| format!("`{item}`")).collect::<Vec<_>>().join(", "));
    }
    if !untouched.is_empty() {
        let _ = writeln!(text, "left to touch: {}, which the card names and no run has changed", untouched.join(", "));
    }
    // the tail, as a cut-off run's: the latest words say where the worker was, and the whole of
    // a long message is context the fresh session would read in again
    let tail = ra_context::last_message_tail(&session.text);
    if !tail.is_empty() {
        let _ = write!(text, "\nthe worker's last message, its tail:\n{tail}");
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
    // a ranged read is a read of its file
    for entry in reads {
        *counts.entry(ra_context::read_entry(entry).0).or_default() += 1;
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

/// The estimate of a run on a node of `complexity`, when the card names none: what a run
/// reads and writes over its turns, about the median of the 898 Claude Code runs of 2026-11-12
/// (every turn reads the harness's own prompt and the session so far again, so c2 to c4 all ran
/// near 54k a turn; the old 5k to 12k a turn put every run over). c1 120k tokens in 4 turns (30k
/// a turn), c2 320k in 8 (40k), c3 675k in 15 (45k), c4 1.25M in 25 (50k), c5 2.2M in 40 (55k).
/// An out-of-range complexity takes the nearest end.
#[must_use]
pub fn estimate_for(complexity: u8) -> Estimate {
    let (tokens, turns) = match complexity {
        0 | 1 => (120_000, 4),
        2 => (320_000, 8),
        3 => (675_000, 15),
        4 => (1_250_000, 25),
        _ => (2_200_000, 40),
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

/// A run the hard turn cap ended before the worker reported: the failure it reads as, and what
/// the handoff the agent writes for it ([`ra_context::Handoff::cut_off`]) needs, the cap and the
/// worker's last words.
#[derive(Debug)]
struct CutOff {
    failure: String,
    cap: String,
    said: String,
}

impl std::fmt::Display for CutOff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.failure)
    }
}

impl std::error::Error for CutOff {}

/// Why a run's session ended, as the ledger stores it: the cap that ended it, the budget, a
/// failure, or the worker's own answer, refused when it stops blocked on a refused command.
fn end_reason(session: &anyhow::Result<Session>) -> ra_context::EndReason {
    use ra_context::EndReason;
    match session {
        Ok(Session { capped: Some(Capped::Turns(_)), .. }) => EndReason::TurnCap,
        Ok(Session { capped: Some(Capped::Context { .. }), .. }) => EndReason::TokenCeiling,
        Ok(session) => EndReason::of_report(&session.text),
        // the tool loop's hard cap fails the run in words, the harness's as a cut-off
        Err(error) if error.downcast_ref::<CutOff>().is_some() || error.to_string().contains("turns without finishing") => {
            EndReason::TurnCap
        }
        Err(error) if error.downcast_ref::<OverBudget>().is_some() => EndReason::TokenCeiling,
        Err(_) => EndReason::Crash,
    }
}

/// Whether a card in `state` is one the person took out of progress while a run of it went:
/// blocked, cancelled or defined by `rc set`. The run's own states (failed, implemented, needs
/// revision) are never it.
#[must_use]
pub fn ends_live_run(state: TaskState) -> bool {
    matches!(state, TaskState::Blocked | TaskState::Cancelled | TaskState::Defined)
}

/// The line the feed and the walk show for a run the person's `rc set` ended.
#[must_use]
pub fn stopped_line(id: i64, title: &str, state: TaskState) -> String {
    format!("stopped #{id} {title}: set {state} while its run was live; the run ended and the card is not continued or rerun")
}

/// What ended a run at a turn: the card was set out of progress by the person.
#[derive(Debug)]
struct Stopped(TaskState);

impl std::fmt::Display for Stopped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the card was set {} while its run was live", self.0)
    }
}

impl std::error::Error for Stopped {}

/// Whether a card in `state` is the person's to finish, so no run of it starts: blocked or
/// cancelled. Defined is not: it is the state a walk picks a card in.
#[must_use]
pub fn held_by_person(state: TaskState) -> bool {
    matches!(state, TaskState::Blocked | TaskState::Cancelled)
}

/// The line the feed and the walk show for a run that did not start on a card the person holds.
#[must_use]
pub fn not_started_line(id: i64, title: &str, state: TaskState) -> String {
    format!("skipped #{id} {title}: it is {state}; no run started, the card waits for the person")
}

/// The state the person set the card to while its run went, when they did: the run's end
/// leaves it as they set it.
pub async fn card_left(store: &Store, id: i64) -> Option<TaskState> {
    state_now(store, id).await.filter(|state| ends_live_run(*state))
}

/// The card's state as the roadmap holds it now: cancelled when it left the roadmap, since `rc
/// set cancelled` removes it; `None` when the roadmap cannot be read.
async fn state_now(store: &Store, id: i64) -> Option<TaskState> {
    let nodes = store.all().await.ok()?;
    Some(nodes.iter().find(|node| node.id == id).map_or(TaskState::Cancelled, |node| node.state))
}

/// The tool loop of a bare model: the model answers with tool calls, the toolbox executes them in
/// the workspace, the results go back, until the hard cap of `caps` runs out when there is one,
/// or the summed cost crosses `budget` when there is one. The restart cap of `caps` ends the run
/// too, but as a session to continue from: what the model said and the calls it made stand in
/// for its answer, and the card goes on in a fresh session.
async fn tool_loop(
    provider: &dyn ModelProvider,
    worker: &Worker,
    (id, store): (i64, Option<&Store>),
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
    let mut biggest: Vec<(String, usize)> = Vec::new();
    // the prompt tokens before the first write, and the files read with no window
    let mut read_in = 0u64;
    let mut edited = false;
    let mut whole: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    // what the worker said and did, for the handoff when the restart cap ends the run
    let mut said: Vec<String> = Vec::new();
    let mut did: Vec<String> = Vec::new();
    let limit = caps.limit();
    loop {
        turn += 1;
        // a card set blocked, cancelled or defined while it runs ends the run at its next turn
        if let Some(store) = store
            && let Some(state) = card_left(store, id).await
        {
            return Err(Stopped(state).into());
        }
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
            return Ok(Session {
                text,
                usage,
                calls,
                turns: u64::from(restart),
                turn_usage,
                cap: limit,
                capped: Some(Capped::Turns(restart)),
                reads,
                biggest,
                read_in,
                whole_reads: u64::try_from(whole.len()).unwrap_or(u64::MAX),
            });
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
                max_context: None,
                effort: None,
                read_only: false,
            })
            .await
            .map_err(map_error)?;
        // every turn is a completion of its own, so each lands in the ledger as it happens; the
        // turns through the first write carry their prompt as the read-in
        let turn_read_in = if edited { 0 } else { answer.usage.prompt_tokens };
        edited = edited || answer.tool_calls.iter().any(|call| matches!(call.name.as_str(), "write_file" | "edit_file"));
        read_in += turn_read_in;
        worker.record_run(Some(id), &answer.usage, 1, turn_read_in).await;
        add_usage(&mut usage, &answer.usage);
        turn_usage.push(TurnUsage::of(&answer.usage));
        if answer.tool_calls.is_empty() {
            return Ok(Session {
                text: answer.text,
                usage,
                calls,
                turns: u64::from(turn),
                turn_usage,
                cap: limit,
                capped: None,
                reads,
                biggest,
                read_in,
                whole_reads: u64::try_from(whole.len()).unwrap_or(u64::MAX),
            });
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
                && let Some(arguments) = serde_json::from_str::<serde_json::Value>(&call.arguments).ok()
                && let Some(path) = arguments["path"].as_str().map(str::to_owned)
            {
                if arguments.get("offset").is_none() && arguments.get("limit").is_none() {
                    whole.insert(path.clone());
                }
                reads.push(path);
            }
            let result = toolbox.execute(call).await;
            // the largest answer is what a heavy run is noted by
            let chars = result.chars().count();
            biggest.push((call_summary(call), chars));
            biggest.sort_by_key(|entry| std::cmp::Reverse(entry.1));
            biggest.truncate(crate::retro::HEAVY_NAMED);
            messages.push(Message::tool(&call.id, result));
            calls += 1;
        }
    }
}

/// The largest files a harness run read, as `read_file path` with their sizes, biggest first:
/// the harness keeps its tool results out of sight, but a file it read whole is as heavy as the
/// file is. Files gone from the workspace are left out.
fn read_sizes(workspace: Option<&Path>, reads: &[String]) -> Vec<(String, usize)> {
    let Some(workspace) = workspace else { return Vec::new() };
    let files: std::collections::BTreeSet<&String> = reads.iter().collect();
    let mut sizes: Vec<(String, usize)> = files
        .into_iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(workspace.join(path)).ok()?;
            Some((format!("read_file {path}"), text.chars().count()))
        })
        .collect();
    sizes.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    sizes.truncate(crate::retro::HEAVY_NAMED);
    sizes
}

/// One ledger row of a completion's usage, under `model` and `role`, with the read-in of a run.
fn entry_of(model: &str, role: Option<&str>, node: Option<i64>, usage: &Usage, turns: u64, read_in: u64) -> Entry {
    Entry {
        model: model.to_owned(),
        role: role.map(str::to_owned),
        node,
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        cached_tokens: usage.cached_tokens,
        cost_usd: usage.cost_usd,
        cost_computed: usage.cost_computed,
        turns,
        read_in,
    }
}

/// Adds one completion's usage to a running sum.
fn add_usage(sum: &mut Usage, one: &Usage) {
    sum.prompt_tokens += one.prompt_tokens;
    sum.completion_tokens += one.completion_tokens;
    sum.cached_tokens += one.cached_tokens;
    if let Some(cost) = one.cost_usd {
        sum.cost_usd = Some(sum.cost_usd.unwrap_or(0.0) + cost);
        // a sum with an estimate in it is an estimate
        sum.cost_computed |= one.cost_computed;
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
            max_context: None,
            effort: None,
            read_only: false,
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

/// The mark of an idle run on its handoff's check line, last on the line: ` · idle: <reason>`.
/// The line is kept in the file, so the next run of the card starts fresh and a second idle run
/// parks it.
const IDLE_MARK: &str = " · idle: ";

/// The last run's handoff of a task, past the notes other cards left on it.
fn last_run(task: &ra_context::TaskContext) -> Option<&ra_context::Handoff> {
    task.handoffs.iter().rev().find(|h| !h.outcome.starts_with("note from"))
}

/// Why the run of a handoff was idle, as its check line carries it; `None` for a run that was not.
fn idle_of(handoff: &ra_context::Handoff) -> Option<&str> {
    handoff.check.split_once(IDLE_MARK).map(|(_, reason)| reason.trim())
}

/// Why the run of `last` was idle: it changed no file, and its report (or the notes it left)
/// names a refused command, or it drew the verdict the run of `previous` drew. `None` for a run
/// that changed a file or says nothing of why it did not.
fn idle_reason(last: &ra_context::Handoff, previous: Option<&ra_context::Handoff>, notes: &str) -> Option<String> {
    if !last.changed.is_empty() {
        return None;
    }
    if let Some(line) = refused_line(&last.report).or_else(|| refused_line(notes)) {
        return Some(line);
    }
    let verdict = last.verdict.as_deref()?;
    (previous.and_then(|p| p.verdict.as_deref()) == Some(verdict)).then(|| format!("the same verdict again: {}", verdict.trim()))
}

/// The first `refused:` line of a report, with the line under it when the label stands alone,
/// clipped to 200 characters; `None` when the report names no refusal.
fn refused_line(report: &str) -> Option<String> {
    let bare = |line: &str| line.trim().trim_start_matches(['-', '*', '#', '>', '`', ' ']).to_owned();
    let mut lines = report.lines().map(bare);
    let rest = lines.find_map(|line| line.strip_prefix("refused:").map(|rest| rest.trim_start_matches(['*', ' ']).trim().to_owned()))?;
    // a label alone, `refused:` or **refused:**, names its refusal on the line under it
    let alone = rest.trim_matches(['`', '*']).is_empty();
    let what = if alone { lines.find(|line| !line.is_empty()).unwrap_or_default() } else { rest };
    let line = format!("refused: {what}");
    Some(line.trim_end().chars().take(200).collect())
}

/// Settles a run of card `id` that its check or its reviewer sent back: one that changed nothing
/// and says why ([`idle_reason`]) is marked idle on its handoff, and when the run before it was
/// idle too the card is blocked, the first run's reason at the top of its notes, and the feed
/// says so ([`crate::pair::parked`]). The line for the run, `None` for a run that was not idle.
async fn settle_idle(store: &Store, root: &Path, id: i64) -> Option<String> {
    let node = store.read(id).await.ok()?;
    if node.state != TaskState::NeedsRevision {
        return None;
    }
    let mut task = ra_context::TaskContext::open(root, id);
    let runs: Vec<usize> = task.handoffs.iter().enumerate().filter(|(_, h)| !h.outcome.starts_with("note from")).map(|(i, _)| i).collect();
    let (&at, before) = runs.split_last()?;
    let previous = before.last().map(|&i| &task.handoffs[i]);
    let notes = node.notes.as_deref().unwrap_or_default();
    // a round no rerun can move is held on its first sight: 730 reruns of three turns or fewer
    // changed nothing and were sent back again (the feeds, 2026-11-12)
    let held = held_round(&task.handoffs[at], previous, notes);
    let reason = held.clone().or_else(|| idle_reason(&task.handoffs[at], previous, notes))?;
    // the first idle run's reason names the card's trouble; the second only repeats it
    let first = if held.is_some() { Some(reason.clone()) } else { previous.and_then(idle_of).map(str::to_owned) };
    let last = &mut task.handoffs[at];
    if idle_of(last).is_none() {
        last.check = format!("{}{IDLE_MARK}{reason}", last.check.trim());
        if let Err(e) = task.save(root) {
            tracing::warn!("the idle run of #{id} was not marked on its handoff: {e:#}");
        }
    }
    let Some(first) = first else {
        return Some(format!("  idle: #{id} changed nothing ({reason}); its next run starts a fresh session"));
    };
    let notes = node.notes.as_deref().map_or_else(|| first.clone(), |notes| format!("{first}\n\n{notes}"));
    if let Err(e) = store.set_notes(id, &notes).await {
        tracing::warn!("the reason #{id} is parked was not noted: {e:#}");
    }
    if let Err(e) = store.set_state_by(id, TaskState::Blocked, Source::Run).await {
        tracing::warn!("#{id} could not be parked: {e:#}");
        return None;
    }
    let text = if held.is_some() { crate::pair::held(root, id, &first) } else { crate::pair::parked(root, id, &first) };
    tracing::warn!("{text}");
    Some(format!("  {text}; a person sets it defined again"))
}

/// Drives [`held_round`] for an integration test.
#[doc(hidden)]
#[must_use]
pub fn held_round_hook(last: &ra_context::Handoff, previous: Option<&ra_context::Handoff>, notes: &str) -> Option<String> {
    held_round(last, previous, notes)
}

/// Why the run of `last` holds its card on this round, before any second idle run: it changed
/// no file, and its report stands under `## Blocked`, says only a person can move the card, or
/// it drew the reviewer's points the run of `previous` drew. `None` leaves the round to
/// [`idle_reason`].
fn held_round(last: &ra_context::Handoff, previous: Option<&ra_context::Handoff>, notes: &str) -> Option<String> {
    if !last.changed.is_empty() {
        return None;
    }
    if let Some(reason) = blocked_reason(&last.report) {
        return Some(format!("the worker reported it blocked: {reason}"));
    }
    if let Some(line) = person_line(&last.report) {
        return Some(format!("only a person can move it: {line}"));
    }
    if let Some(line) = missing_tool_line(&last.report) {
        return Some(format!("a tool is missing, only a person can install it: {line}"));
    }
    let points = last.points()?;
    if previous.and_then(ra_context::Handoff::points) != Some(points) {
        return None;
    }
    // the first point names what the reviewer keeps asking for
    let first = ra_context::review_of(notes).and_then(|review| review.lines().nth(1).map(|p| p.trim_start_matches("- ").to_owned()));
    let first: String = first.unwrap_or_default().chars().take(200).collect();
    Some(format!("the reviewer's points again on an unchanged tree: {first}").trim_end_matches([':', ' ']).to_owned())
}

/// The turns a session spends reading in, a few past the harness's own reading-in allowance:
/// a context cap hit within them says the card's files alone fill the context, and a restart
/// would only read them again.
const READING_IN_TURNS: u32 = 20;

/// Whether the card's last session ended on the context cap within its reading-in turns, so
/// this run goes without the cap.
fn capped_reading_in(task: &ra_context::TaskContext) -> bool {
    let Some(last) = task.handoffs.last() else { return false };
    if last.outcome != "continues" || !last.check.contains("the context cap") {
        return false;
    }
    // the report opens "the context cap of … ended the run after N turn(s)"
    let turns = last
        .report
        .split_once(" ended the run after ")
        .and_then(|(_, rest)| rest.split_whitespace().next())
        .and_then(|n| n.parse::<u32>().ok());
    turns.is_some_and(|n| n <= READING_IN_TURNS)
}

/// Whether a run of `node` is a rerun: the card came back from its check or its reviewer, or a
/// fix pass is on it. A rerun reads the rerun block as its prompt, not the handoffs, in a fresh
/// session unless the card is absolute ([`rerun_resume`]). The walk reruns a first rejection and the first two
/// failed checks by itself; past that the person re-queues the card with `rc set <id> defined`,
/// which erases the state but not the reviewer's block at the head of the notes,
/// which stands until the next run writes its report: a card whose notes open with a
/// `needs revision` review is a rerun whatever its state (rhipper-asm #47: the five points
/// were in the notes and not in the prompt, and the worker spent its first turns finding them).
fn is_rerun(node: &Node, worker: &Worker) -> bool {
    worker.fix.is_some() || sent_back(node)
}

/// Whether the card itself says it was sent back, whoever runs it: the half of [`is_rerun`]
/// the context preview can tell without a worker.
fn sent_back(node: &Node) -> bool {
    node.state == TaskState::NeedsRevision
        || ra_context::review_of(node.notes.as_deref().unwrap_or_default())
            .is_some_and(|review| review.lines().next().is_some_and(|first| first.contains("needs revision")))
}

/// The fingerprint of the words a review of `node` judges, its title, objective and acceptance;
/// a card without acceptance keeps the print of its title and objective.
#[must_use]
pub fn card_print(node: &Node) -> String {
    let objective = node.objective.as_deref().unwrap_or_default();
    match node.acceptance.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
        Some(acceptance) => ra_context::brief_fingerprint(&node.title, &format!("{objective}\nAcceptance: {acceptance}")),
        None => ra_context::brief_fingerprint(&node.title, objective),
    }
}

/// The notes of `node` a review reads, and the points of earlier reviews dropped from them
/// because the card's text changed since ([`crate::review::without_stale_points`]).
fn review_notes(project: &Path, node: &Node) -> (String, usize) {
    let task = ra_context::TaskContext::open(project, node.id);
    let judged = task.handoffs.last().and_then(ra_context::Handoff::brief);
    crate::review::without_stale_points(node.notes.as_deref().unwrap_or_default(), judged, &card_print(node))
}

/// The points a rerun carries: the failures and the warnings its last run left and the lines
/// of the review; they rank the map's items as they name them, and the quote holds those.
fn rerun_points(review: Option<&str>, last: Option<&ra_context::Handoff>) -> Vec<String> {
    let left = last.into_iter().flat_map(|last| last.failures.iter().chain(&last.warnings).cloned());
    left.chain(review.iter().flat_map(|r| r.lines().map(str::to_owned))).collect()
}

/// How many lines of the failing check a rerun reads: the failures lead the block, so what is
/// past this is the noise around them.
const RERUN_CHECK_LINES: usize = 30;

/// The session a run resumes: [`resume_of`]'s, except that a rerun of a card that is not
/// absolute starts fresh. The rerun block carries what the rerun needs, and a resumed session
/// paid its whole transcript every turn of a one-line answer (#210: about 990 heavy-turn lines
/// at 26k to 86k tokens a turn); an absolute card keeps its session, its work being one run.
fn rerun_resume(node: &Node, rerun: bool, now: u64) -> Result<Option<String>, String> {
    match resume_of(node, now) {
        Ok(Some(_)) if rerun && !node.is_absolute() => Err("a rerun starts a fresh session".to_owned()),
        other => other,
    }
}

/// How many lines of the last run's report a rerun reads.
const RERUN_REPORT_LINES: usize = 20;

/// What a rerun reads in place of the handoffs, from [`rerun_text`], with what changed in the
/// tree it acts in and in the run's environment since the last run.
fn rerun_block(task: &ra_context::TaskContext, node: &Node, acting: &Path, root: &Path) -> Option<String> {
    let mut since = task.handoffs.last().map(|last| since_last_run(acting, root, node.id, last.when)).unwrap_or_default();
    since.extend(behind_main(acting, &node.files));
    if let Some(last) = task.handoffs.last() {
        since.extend(commits_touching_failures(acting, last, &node.files));
    }
    rerun_text(task, node, &since)
}

/// How many commits a rerun's block lists for a failed test's files.
const FAILURE_COMMITS: usize = 10;

/// The source paths a failure text names: a panic's `file.rs:12:5` and a compiler's `--> file.rs:3:1`.
fn failure_paths(failures: &[String]) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    for word in failures.iter().flat_map(|f| f.split_whitespace()) {
        let path = word.trim_matches(|c: char| matches!(c, '"' | '\'' | '(' | ')' | ',')).replace('\\', "/");
        let path = path.split(".rs").next().filter(|_| path.contains(".rs")).map(|stem| format!("{stem}.rs"));
        if let Some(path) = path.filter(|p| p.len() > 3 && !p.contains(':') && !p.starts_with('/'))
            && !paths.contains(&path)
        {
            paths.push(path);
        }
    }
    paths
}

/// The commits since the last run (`last`) that touch the file of a test it failed or the card's
/// `files`, as hash and subject, newest first, at most [`FAILURE_COMMITS`]; nothing when the run
/// failed no test or the tree has no such commit.
pub fn commits_touching_failures(acting: &Path, last: &ra_context::Handoff, files: &[String]) -> Vec<String> {
    let mut paths = failure_paths(&last.failures);
    if paths.is_empty() {
        return Vec::new();
    }
    paths.extend(files.iter().map(|f| f.trim_start_matches('+').replace('\\', "/")));
    paths.sort();
    paths.dedup();
    let mut args = vec!["log", "--format=%h %ct %s", "-n", "200", "--"];
    args.extend(paths.iter().map(String::as_str));
    let Some(log) = std::process::Command::new("git")
        .args(&args)
        .current_dir(acting)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
    else {
        return Vec::new();
    };
    log.lines()
        .filter_map(|line| {
            let (hash, rest) = line.split_once(' ')?;
            let (at, subject) = rest.split_once(' ')?;
            (at.parse::<u64>().ok()? > last.when)
                .then(|| format!("commit {hash} {subject} (touches a failed test's file or the card's files)"))
        })
        .take(FAILURE_COMMITS)
        .collect()
}

/// How far the tree a card acts in is behind `main`: the count of commits it lacks, the subjects
/// of those that touch the card's `files` or their crates, and the order to merge main first.
/// Nothing when the tree is level with main or has no `main`.
pub fn behind_main(acting: &Path, files: &[String]) -> Vec<String> {
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(acting)
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let Some(count) = git(&["rev-list", "--count", "HEAD..main"]).and_then(|n| n.trim().parse::<usize>().ok()).filter(|n| *n > 0) else {
        return Vec::new();
    };
    // a crate is the first two components of a path under `crates/` or `apps/`
    let mut paths: Vec<String> = Vec::new();
    for file in files {
        let file = file.trim_start_matches('+').replace('\\', "/");
        let parts: Vec<&str> = file.split('/').collect();
        if matches!(parts.first(), Some(&"crates" | &"apps")) && parts.len() > 2 {
            paths.push(parts[..2].join("/"));
        }
        paths.push(file);
    }
    paths.sort();
    paths.dedup();
    let mut lines =
        vec![format!("this tree is {count} commit{} behind main; merge main first, before any change", if count == 1 { "" } else { "s" })];
    if !paths.is_empty() {
        let mut args = vec!["log", "--format=%s", "-n", "20", "HEAD..main", "--"];
        args.extend(paths.iter().map(String::as_str));
        let log = git(&args).unwrap_or_default();
        lines.extend(log.lines().filter(|s| !s.trim().is_empty()).map(|s| format!("main commit touching the card's files or crates: {s}")));
    }
    lines
}

/// What a rerun reads in place of the handoffs: the last run's check line, its verdict and the
/// files it changed; every failure of the failed check whole, as the handoff kept them from the
/// whole output, each marked when the base fails it too; the failing check's tail from the head
/// of the notes, clipped; what changed since (`since`); and the last run's own report. The
/// reviewer's points ride verbatim in the context's Review section. `None` when no run is
/// behind the card.
fn rerun_text(task: &ra_context::TaskContext, node: &Node, since: &[String]) -> Option<String> {
    let last = task.handoffs.last()?;
    let mut text = format!("The last run of this card came back; take it up on these lines, not afresh:\ncheck: {}\n", last.check.trim());
    if let Some(verdict) = &last.verdict {
        writeln!(text, "verdict: {}", verdict.trim()).unwrap();
    }
    if !last.changed.is_empty() {
        writeln!(text, "changed: {}", last.changed.join(" ")).unwrap();
    }
    if !last.failures.is_empty() {
        writeln!(
            text,
            "\nThe failures of the check, every one, read from its whole output; a name marked \"{}\" fails on the project's tree too, so it is not this card's to fix:\n{}",
            ra_context::ON_BASE.trim_start_matches(" · "),
            last.failures.join("\n\n")
        )
        .unwrap();
    }
    // the lint's warnings sit in the card's own lines on the merged branch; a worktree cannot
    // read the notes where they were first, so each rides here whole, file, line and message
    if !last.warnings.is_empty() {
        writeln!(text, "\nThe lint's warnings in the lines this card changed, every one:\n{}", last.warnings.join("\n\n")).unwrap();
    }
    // the reviewer's block is the task context's Review section, quoted whole; the failing
    // check's output goes here
    for block in judgment_of(node.notes.as_deref().unwrap_or_default()) {
        if !block.starts_with("review by ") {
            writeln!(text, "\n{block}").unwrap();
        }
    }
    if !since.is_empty() {
        let lines: Vec<String> = since.iter().map(|line| format!("- {line}")).collect();
        writeln!(text, "\nWhat changed since the last run:\n{}", lines.join("\n")).unwrap();
    }
    let report = without_judgment(&last.report);
    let lines: Vec<&str> = report.lines().collect();
    if !lines.is_empty() {
        let shown = &lines[..lines.len().min(RERUN_REPORT_LINES)];
        writeln!(text, "\nThe last run's report:\n{}", shown.join("\n")).unwrap();
        if lines.len() > shown.len() {
            writeln!(text, "… {} more lines in the notes of the card", lines.len() - shown.len()).unwrap();
        }
    }
    Some(text)
}

/// A report without the judgment at its head: the reviewer's block and the checks' output,
/// which the rerun block gives on its own lines.
fn without_judgment(report: &str) -> String {
    let lines: Vec<&str> = report.lines().collect();
    let mut at = 0;
    loop {
        while lines.get(at).is_some_and(|l| l.trim().is_empty()) {
            at += 1;
        }
        let Some(line) = lines.get(at) else { break };
        if line.starts_with("review by ") {
            at += 1;
            while lines.get(at).is_some_and(|l| l.starts_with("- ")) {
                at += 1;
            }
        } else if line.starts_with("check failed: ") || line.starts_with("check passed: ") {
            // a check's output runs to its exit line; one that could not run, to its blank line
            let rest = &lines[at..];
            let exit = rest.iter().position(|l| l.starts_with("exit code ") || l.starts_with("timed out after "));
            let blank = rest.iter().position(|l| l.trim().is_empty());
            at += exit.map(|e| e + 1).or(blank).unwrap_or(rest.len());
        } else {
            break;
        }
    }
    lines[at..].join("\n").trim().to_owned()
}

/// The file of a card's run environment: the PATH and the project's allowlist as the last run
/// had them, for the next run to say what moved.
fn stamp_path(root: &Path, id: i64) -> PathBuf {
    root.join(ra_context::CONTEXT_DIR).join(format!("{id}.env"))
}

/// The run environment now: `PATH=…` and `allow=…`, one line each.
fn environment_stamp(root: &Path) -> String {
    let path = std::env::var("PATH").unwrap_or_default();
    let allow = crate::router::Router::load(root).map(|router| router.worker.allow.join("\u{1f}")).unwrap_or_default();
    format!("PATH={path}\nallow={allow}\n")
}

/// Keeps the run environment of card `id` for its next run; a stamp not written only costs
/// that run its environment lines.
fn save_stamp(root: &Path, id: i64) {
    let path = stamp_path(root, id);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&path, environment_stamp(root)) {
        tracing::warn!("the run environment of #{id} was not kept: {e:#}");
    }
}

/// What changed since the last run of card `id`, which ended at `when`: the merges, resets and
/// rebases of the tree it acts in, from its reflog, and the PATH entries and allow rules the
/// run environment gained or lost since its stamp.
fn since_last_run(acting: &Path, root: &Path, id: i64, when: u64) -> Vec<String> {
    let mut lines = Vec::new();
    let reflog = std::process::Command::new("git")
        .args(["log", "-g", "--date=unix", "--format=%gd %gs", "-n", "50"])
        .current_dir(acting)
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
        .unwrap_or_default();
    lines.extend(reflog_since(&reflog, when));
    if let Ok(stamp) = std::fs::read_to_string(stamp_path(root, id)) {
        lines.extend(environment_moved(&stamp, &environment_stamp(root)));
    }
    lines
}

/// The entries of a `git log -g --date=unix --format="%gd %gs"` output after `when` that move
/// the tree under a run: a merge, a reset, a rebase, a pull; oldest first.
fn reflog_since(reflog: &str, when: u64) -> Vec<String> {
    const MOVES: [&str; 4] = ["merge", "reset", "rebase", "pull"];
    let mut moved: Vec<String> = reflog
        .lines()
        .filter_map(|line| {
            let (selector, subject) = line.split_once(' ')?;
            let at: u64 = selector.split_once("@{")?.1.strip_suffix('}')?.parse().ok()?;
            (at > when && MOVES.iter().any(|word| subject.starts_with(word))).then(|| subject.trim().to_owned())
        })
        .collect();
    moved.reverse();
    moved
}

/// What moved between the stamp of the last run and the one now: the PATH entries and the
/// allow rules gained and lost, one line each.
fn environment_moved(before: &str, now: &str) -> Vec<String> {
    let field = |stamp: &str, key: &str| -> Vec<String> {
        let value = stamp.lines().find_map(|line| line.strip_prefix(key)).unwrap_or_default();
        match key {
            "PATH=" => std::env::split_paths(value).map(|p| p.display().to_string()).filter(|p| !p.is_empty()).collect(),
            _ => value.split('\u{1f}').filter(|rule| !rule.is_empty()).map(str::to_owned).collect(),
        }
    };
    let mut lines = Vec::new();
    for (key, what) in [("PATH=", "the PATH"), ("allow=", "the allowlist")] {
        let (old, new) = (field(before, key), field(now, key));
        let gained: Vec<&str> = new.iter().filter(|e| !old.contains(*e)).map(String::as_str).collect();
        let lost: Vec<&str> = old.iter().filter(|e| !new.contains(*e)).map(String::as_str).collect();
        if !gained.is_empty() {
            lines.push(format!("{what} gained {}", gained.join(", ")));
        }
        if !lost.is_empty() {
            lines.push(format!("{what} lost {}", lost.join(", ")));
        }
    }
    lines
}

/// The latest reviewer's block at the head of a card's notes, which a run that fails its check
/// carries onto the notes it leaves: the points stand until the card is judged again.
fn carried_review(notes: Option<&str>) -> Option<String> {
    judgment_of(notes.unwrap_or_default()).into_iter().find(|block| block.starts_with("review by "))
}

/// The judgment at the head of a card's notes: the reviewer's verdict with its points, and the
/// failing check's command and output up to its exit line, each a block, the check's clipped to
/// [`RERUN_CHECK_LINES`] with its first and last lines kept. Nothing once the notes reach the
/// checks that passed or the worker's own report.
pub(crate) fn judgment_of(notes: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut lines = notes.lines().peekable();
    while let Some(first) = lines.next() {
        if first.starts_with("review by ") {
            let mut block = vec![first];
            while let Some(point) = lines.next_if(|l| l.starts_with("- ")) {
                block.push(point);
            }
            blocks.push(block.join("\n"));
        } else if first.starts_with("check failed: ") {
            let mut block = vec![first];
            // the output ends at its exit line, or at the line that says the limit stopped it;
            // a command that could not run has neither, so the scan also stops where the next
            // block starts, and past what the notes keep of a failed check (its first failure
            // and the tail) regardless
            for line in lines.by_ref() {
                if line.starts_with("check passed: ") || block.len() >= 3 * TAIL_LINES {
                    break;
                }
                block.push(line);
                if line.starts_with("exit code ") || line.starts_with("timed out after ") {
                    break;
                }
            }
            blocks.push(clip_middle(&block));
        } else if !first.trim().is_empty() && !is_left_point(first) {
            // the points that left the card are not the rerun's, and the judgment goes on past them
            break;
        }
    }
    blocks
}

/// `lines` as a text, whole under [`RERUN_CHECK_LINES`], else their head and tail around a line
/// that counts what is left in the notes of the card.
fn clip_middle(lines: &[&str]) -> String {
    if lines.len() <= RERUN_CHECK_LINES {
        return lines.join("\n");
    }
    let half = RERUN_CHECK_LINES / 2;
    let dropped = lines.len() - 2 * half;
    format!("{}\n… {dropped} more lines in the notes of the card\n{}", lines[..half].join("\n"), lines[lines.len() - half..].join("\n"))
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
const SYSTEM: &str = "You are an implementation worker of a roadmap-driven orchestrator. You receive one task with its objective, acceptance criteria and related context. When you have tools, implement the task in the working directory and answer with a short report of what you changed, naming the file each change landed in in backticks, and what you verified. The code comes first and the report is the deliverable: the orchestrator compiles and checks your change after your report, on a build slot of its own, and a failure comes back to you with its failures and the review's points as a rerun. So do not build to explore, do not build before you have changed anything, and do not build at all when reading the code you changed and what it calls tells you it is right; build once, at the end, only when a compile is in doubt (a new API, a type you did not see defined, a generic bound), and never more than twice in a session. A change that is right by reading, reported without a build, is the normal case, not a shortcut; say in the report what you verified by reading and what the check will prove. Bash and PowerShell both run the build tools, the read-only git commands and the read-only listing commands; pipe a long output through tail. After your report the orchestrator lints the files you changed, and any warning comes back to you; do not hunt warnings yourself. Without tools, answer with a plan of the implementation and the code or the changes it names, nothing else.";

/// What a worker is told in a project whose `[worker] builds` is zero: the orchestrator builds,
/// the worker writes.
const NO_BUILDS: &str = "In this project you do not build: every gradle, cargo or other build command is refused, and six workers write at once while the orchestrator compiles on two build slots of its own. Research the code the card touches, write the change, read it against what it calls, and report it; the orchestrator's check compiles your report, and a compile failure comes back to this session with the compiler's output for you to fix the same way. Your report says what you verified by reading and what the check will prove.";

/// The system prompt of a reassessment.
const ARCHITECT: &str = "You are the architect of a roadmap-driven orchestrator. You receive one roadmap node with its context. You keep the objective and the acceptance short, one topic per sentence, and you name the remaining work, not the work already done. When the objective and the acceptance already name only the remaining work and are checkable, you answer with the same text rather than rewording it.";

/// The brief of a card: the prompt text, and the files it works in, which the outlines read.
struct Brief {
    text: String,
    /// The card's files, or those it inherits from the nearest ancestor that names any.
    files: Vec<String>,
}

/// The line an ancestor adds to the brief: its objective's first sentence, nothing without one.
fn ancestor_line(ancestor: &Node) -> String {
    let Some(objective) = ancestor.objective.as_deref().map(str::trim).filter(|o| !o.is_empty()) else {
        return String::new();
    };
    let end = objective
        .char_indices()
        .find(|&(i, c)| matches!(c, '.' | '!' | '?' | '\n') && objective[i + c.len_utf8()..].chars().next().is_none_or(char::is_whitespace))
        .map_or(objective.len(), |(i, c)| if c == '\n' { i } else { i + c.len_utf8() });
    format!("Under #{} {}: {}\n", ancestor.id, ancestor.title, &objective[..end])
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
    // the files where the symbols the card names are defined or called, so the worker opens
    // them by name rather than searching the tree for them
    let named =
        format!("{}\n{}\n{}", node.title, node.objective.as_deref().unwrap_or_default(), node.acceptance.as_deref().unwrap_or_default());
    let also = ra_context::also_read::also_read(&workspace_of(store), &named, files);
    if !also.is_empty() {
        text.push_str("Also read, where the symbols the card names are defined or used:\n");
        for file in also {
            let _ = writeln!(text, "  {file}");
        }
    }
    // a symbol the card names that no file defines, and the open card that adds it: the worker
    // does not search for what is not there
    for symbol in ra_context::also_read::missing_symbols(&workspace_of(store), &named) {
        let _ = write!(text, "Not defined in the tree: `{symbol}`");
        let adder = nodes.iter().find(|n| {
            n.id != node.id
                && !matches!(n.state, TaskState::Accepted | TaskState::Cancelled)
                && ra_context::also_read::symbols_of(&format!("{}\n{}", n.title, n.objective.as_deref().unwrap_or_default()))
                    .contains(&symbol)
        });
        match adder {
            Some(card) => {
                let _ = writeln!(text, "; the open card #{} {} [{}] adds it", card.id, card.title, card.state);
            }
            None => text.push('\n'),
        }
    }
    // the ancestors: the chain of parents above the task, nearest first
    let mut parent = node.parent.and_then(find);
    while let Some(ancestor) = parent {
        text.push_str(&ancestor_line(ancestor));
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
            text.push_str(&dependency_notes_line(dep.id, notes));
        }
    }
    text.push_str(context_cancel_line::CANCEL_LINE);
    Ok(Brief { text, files: files.to_vec() })
}

/// The brief's line of a dependency's notes, cut to their head, where the review sits: asm
/// #147's brief carried #86's whole notes, a 1.5 MB clippy dump, and every turn of its run
/// was 780k tokens (2026-11-15).
fn dependency_notes_line(id: i64, notes: &str) -> String {
    let end = notes.char_indices().nth(DEPENDENCY_NOTES_ROOM).map_or(notes.len(), |(at, _)| at);
    let more = if end < notes.len() { " …(clipped)" } else { "" };
    format!("  notes of #{id}: {}{more}\n", &notes[..end])
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
        ProviderError::ContextOut { completion, cap, size } => anyhow::anyhow!(
            "the run failed: the context cap of {cap} tokens ended the run at a turn of {size} after {} turn(s) without finishing",
            completion.turns
        ),
        ProviderError::Other(text) => anyhow::anyhow!("the run failed: {text}"),
    }
}

/// A cost for a log line: the dollars the provider reported, the list price computed from the
/// tokens marked as an estimate (`$4.12 est.`), or a word when there is neither.
fn cost_text(usage: &Usage) -> String {
    match usage.cost_usd {
        Some(c) if usage.cost_computed => format!("${c:.2} est."),
        cost => dollars(cost),
    }
}

/// The cost of a run's end line with the cached share of its input beside it, `$4.12 est. ·
/// 90% cached`: the share is what the cache rate priced, and what a fresh session gives up.
fn end_cost(usage: &Usage) -> String {
    let cached = usage.cached_tokens * 100 / usage.prompt_tokens.max(1);
    format!("{} · {cached}% cached", cost_text(usage))
}

/// A bare cost for a log line: the dollars, or a word when there are none.
fn dollars(cost: Option<f64>) -> String {
    cost.map_or_else(|| "cost unreported".to_owned(), |c| format!("${c:.4}"))
}

/// The model id without its vendor.
pub(crate) fn short(model: &str) -> &str {
    model.rsplit('/').next().unwrap_or(model)
}

mod card_check_timeout;
mod context_cancel_line;
#[cfg(test)]
mod tests;
mod turn_cap_warning;
pub use card_check_timeout::refuse_timeout;
pub use turn_cap_warning::turn_cap_warning;
