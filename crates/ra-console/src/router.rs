//! The router: the model of a card when neither the card nor the command names one. The rules
//! are the `[[route]]` tables of `rhipper.toml` beside the roadmap, read in order; the first that
//! fits the card's complexity, its kind and the role asked for names the model and, when it says
//! so, the effort. A card's own model wins over every rule; no file, or no rule that fits, is the
//! default model.

use std::fmt;
use std::path::Path;

use anyhow::Context as _;
use ra_roadmap::model::{Effort, Node, NodeKind};
use serde::Deserialize;

/// The rules file, beside the roadmap.
pub const ROUTES_FILE: &str = "rhipper.toml";
/// The role of the worker that implements a card; a rule without a role is for it.
pub const IMPLEMENTER: &str = "implementer";

/// The role of the second model that reads a card's diff against its acceptance.
pub const REVIEWER: &str = "reviewer";

/// The role of the architect that splits a milestone or a section into cards.
pub const PLANNER: &str = "planner";

/// The complexities a rule is for: one, or a range with both ends in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum Span {
    One(u8),
    Between(u8, u8),
}

impl Span {
    /// Whether `complexity` is in the span.
    #[must_use]
    pub fn holds(self, complexity: u8) -> bool {
        match self {
            Self::One(c) => c == complexity,
            Self::Between(low, high) => (low.min(high)..=low.max(high)).contains(&complexity),
        }
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::One(c) => write!(f, "c{c}"),
            Self::Between(low, high) => write!(f, "c{low}..c{high}"),
        }
    }
}

/// One `[[route]]` table: what it is for, and the model and effort it names. A field left out
/// fits every card.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Rule {
    pub complexity: Option<Span>,
    pub kind: Option<NodeKind>,
    #[serde(default = "implementer")]
    pub role: String,
    pub model: String,
    pub effort: Option<Effort>,
}

fn implementer() -> String {
    IMPLEMENTER.to_owned()
}

impl Rule {
    /// Whether the rule is for `node` run in `role`.
    #[must_use]
    pub fn fits(&self, node: &Node, role: &str) -> bool {
        self.role.eq_ignore_ascii_case(role)
            && self.complexity.is_none_or(|span| span.holds(node.complexity))
            && self.kind.is_none_or(|kind| kind == node.kind)
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut terms = Vec::new();
        if let Some(span) = self.complexity {
            terms.push(span.to_string());
        }
        if let Some(kind) = self.kind {
            terms.push(kind.to_string());
        }
        terms.push(self.role.clone());
        write!(f, "{} → {}", terms.join(" · "), self.model)?;
        if let Some(effort) = self.effort {
            write!(f, " (effort {effort})")?;
        }
        Ok(())
    }
}

/// The `[worker]` table: what a project's workers get beyond the defaults.
///
/// ```toml
/// [worker]
/// allow = ["rhtest", "Bash(ninja *)"]   # commands the worker may run here, on top of the defaults
/// check = ".\\gw.ps1 :core:test"         # the check of a card whose acceptance names none
/// lint = ""                             # the lint after a passed check; empty is none, absent is clippy
/// turns = 20                            # a run ends after this many turns and the card goes on in a fresh session
/// context = 120000                      # and after a turn whose input passes this many tokens; 120000 without it
/// check_timeout = 1800                 # the seconds a check or the lint may run; 1200 without it
/// parallel = 2                         # the cards a walk runs at once, in worktrees sharing one build; one without it
/// carry = ["local.properties"]         # untracked files a new worktree gets a copy of
/// borrow = false                       # no borrowing of the pool's spare Opus slots past `parallel`; true without it
/// [worker.env]
/// CARGO_BUILD_JOBS = "6"               # the environment its harness runs with
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct WorkerConfig {
    /// Extra permission rules: a Claude Code rule such as `Bash(ninja *)`, or a bare command
    /// prefix such as `rhtest`, which stands for the command and its arguments in both shells.
    #[serde(default)]
    pub allow: Vec<String>,
    /// The environment the worker's harness runs with, on top of the agent's own.
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
    /// The check of a card whose acceptance names no command; the workspace tests without it.
    #[serde(default)]
    pub check: Option<String>,
    /// The lint of what a run changed, once its check passed: absent, clippy over the changed
    /// crates, as a Rust project wants; empty, no lint for this project; else this command.
    #[serde(default)]
    pub lint: Option<String>,
    /// The turns a run gets before it ends with a handoff and the walk starts the card again in
    /// a fresh session, so the context every turn carries starts small again; absent or zero, a
    /// run goes on until it answers.
    #[serde(default)]
    pub turns: Option<u32>,
    /// The input tokens a turn may carry before the run ends the same way. Absent or zero, no
    /// cap: the harness compacts its own context as it goes, and a forced restart throws away
    /// what the session holds (the person: "claude manages context on its own … let it keep
    /// going unless we have a need to /clear"). A project that wants the restart names a count.
    #[serde(default)]
    pub context: Option<u64>,
    /// The seconds a card's check, or its lint, may run before it is stopped and counts as
    /// failed; absent or zero, the agent's twenty minutes. A suite that builds and runs every
    /// reference program outgrows that (rhipper-asm #69), and the project says so here.
    #[serde(default)]
    pub check_timeout: Option<u64>,
    /// The cards a walk of this project runs at once, each in its own worktree, when
    /// `rc walk` names no `--parallel`; absent, zero or one, one after another. The runs share
    /// one build: the project's target directory and the lock on it.
    #[serde(default)]
    pub parallel: Option<u32>,
    /// Files of the project a new worktree gets a copy of: the ones git does not track that a
    /// build needs all the same, such as `local.properties` with the Android SDK's location.
    /// Without it every worktree run of runelite failed its compile at once.
    #[serde(default)]
    pub carry: Vec<String>,
    /// Whether a walk here may borrow the pool's Opus slots no other walk needs, past its
    /// `parallel`; absent, it may. `false` holds a project to its own count exactly.
    #[serde(default)]
    pub borrow: Option<bool>,
}

impl WorkerConfig {
    /// The cards a walk runs at once, when the table names more than one.
    #[must_use]
    pub fn parallel(&self) -> Option<u32> {
        self.parallel.filter(|n| *n > 1)
    }

    /// Whether a walk may borrow the pool's spare Opus slots past its own count.
    #[must_use]
    pub fn borrow(&self) -> bool {
        self.borrow.unwrap_or(true)
    }

    /// How long a check or the lint may run: the table's seconds, else the agent's default.
    #[must_use]
    pub fn check_timeout(&self) -> std::time::Duration {
        self.check_timeout.filter(|secs| *secs > 0).map_or(crate::agent::CHECK_TIMEOUT, std::time::Duration::from_secs)
    }

    /// The project's default check, when the table names a non-empty one.
    #[must_use]
    pub fn check(&self) -> Option<&str> {
        self.check.as_deref().map(str::trim).filter(|c| !c.is_empty())
    }

    /// Whether the project's check is the full suite: none named, which is the workspace tests,
    /// or a `cargo test` over no package. A worker's own run of it is then narrowed to the crates
    /// it changed, as the orchestrator runs the suite after the report.
    #[must_use]
    pub fn checks_the_suite(&self) -> bool {
        self.check().is_none_or(ra_gotchas::is_suite)
    }

    /// The turns after which a run ends and the card continues fresh, when the table names a
    /// positive count.
    #[must_use]
    pub fn turns(&self) -> Option<u32> {
        self.turns.filter(|turns| *turns > 0)
    }

    /// The input tokens a turn may carry before the run ends and the card continues fresh: the
    /// table's count when positive; `None`, no cap, otherwise.
    #[must_use]
    pub fn context(&self) -> Option<u64> {
        self.context.filter(|tokens| *tokens > 0)
    }

    /// The project's lint: `None` leaves the lint to the agent, `Some("")` turns it off.
    #[must_use]
    pub fn lint(&self) -> Option<&str> {
        self.lint.as_deref().map(str::trim)
    }

    /// The permission rules of `allow`: a bare prefix becomes a rule for each shell.
    #[must_use]
    pub fn rules(&self) -> Vec<String> {
        let mut rules = Vec::new();
        for entry in self.allow.iter().map(|e| e.trim()).filter(|e| !e.is_empty()) {
            if entry.contains('(') {
                rules.push(entry.to_owned());
            } else {
                rules.push(format!("Bash({entry} *)"));
                rules.push(format!("Bash({entry})"));
                rules.push(format!("PowerShell({entry} *)"));
                rules.push(format!("PowerShell({entry})"));
            }
        }
        rules
    }

    /// Whether `command` starts with one of the allowed prefixes: such a command is the
    /// project's own, outside the shell allowlist the gotchas guard.
    #[must_use]
    pub fn allows(&self, command: &str) -> bool {
        let command = command.trim_start();
        self.allow
            .iter()
            .filter_map(|e| prefix_of(e))
            .any(|prefix| command.starts_with(&prefix) && command[prefix.len()..].chars().next().is_none_or(char::is_whitespace))
    }
}

/// The command prefix of an allow entry: `rhtest` of `rhtest`, `ninja` of `Bash(ninja *)`.
fn prefix_of(entry: &str) -> Option<String> {
    let entry = entry.trim();
    let inner = match entry.find('(') {
        Some(at) => entry[at + 1..].trim_end_matches(')'),
        None => entry,
    };
    let prefix = inner.trim().trim_end_matches('*').trim_end();
    (!prefix.is_empty()).then(|| prefix.to_owned())
}

/// The `[role]` table: the model of each cheap role a run ends with. A key left out takes the
/// local server when it answers a ping and no model call when it does not; an empty string turns
/// the role off, whatever the server does.
///
/// ```toml
/// [role]
/// summary = "local/qwen3-8b"     # the report condensed for the handoff
/// digest = "local/qwen3-8b"      # the run's line in the project's digest
/// classify = ""                  # whether an untagged look-back is [machine]: off
/// fix = "local/qwen3-8b"         # the fix pass of a trivially rejected card; haiku without a server
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct RoleConfig {
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub digest: Option<String>,
    #[serde(default)]
    pub classify: Option<String>,
    /// The fix pass, the one role with tools: unlike the others it falls to haiku, not to no
    /// call, when no local server answers.
    #[serde(default)]
    pub fix: Option<String>,
}

impl RoleConfig {
    /// The table's word on `role`: `None` left out, `Some("")` off, else the model.
    #[must_use]
    pub fn model(&self, role: ra_providers::local::Role) -> Option<&str> {
        use ra_providers::local::Role;
        match role {
            Role::Summary => &self.summary,
            Role::Digest => &self.digest,
            Role::Classify => &self.classify,
        }
        .as_deref()
        .map(str::trim)
    }
}

/// The `[plan]` table: how far into the week a run on a `claude-*` model may go before it is
/// steered, to the local route when one is up and refused otherwise, unless `--force`.
///
/// ```toml
/// [plan]
/// threshold = 0.95    # the share of the week; 0.9 when left out
/// ```
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct PlanConfig {
    #[serde(default)]
    pub threshold: Option<f64>,
}

impl PlanConfig {
    /// The threshold, 0 to 1: the table's when it names a sane one, else the default.
    #[must_use]
    pub fn threshold(&self) -> f64 {
        self.threshold.filter(|t| (0.0..=1.0).contains(t)).unwrap_or(ra_providers::limits::DEFAULT_THRESHOLD)
    }
}

/// The rules of a project, in the order of the file, its worker table, its role table and its
/// plan table.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Router {
    #[serde(default, rename = "route")]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub worker: WorkerConfig,
    #[serde(default)]
    pub role: RoleConfig,
    #[serde(default)]
    pub plan: PlanConfig,
}

/// Why a choice fell where it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Why {
    /// The card names its own model.
    Card,
    /// The rule at this index, counted from one, fits the card.
    Rule(usize),
    /// Neither the card nor a rule names one.
    Default,
}

/// The model a card runs on, the effort when a rule set one, and the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub model: String,
    pub effort: Option<Effort>,
    pub why: Why,
}

impl Router {
    /// The rules of `text`, a `rhipper.toml`.
    ///
    /// # Errors
    /// When the text is not the rules file: a rule without a model, an unknown kind or effort.
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        toml::from_str(text).context("the routes do not parse")
    }

    /// The rules of the `rhipper.toml` in `root`; none when there is no file.
    ///
    /// # Errors
    /// When the file is there but cannot be read or parsed.
    pub fn load(root: &Path) -> anyhow::Result<Self> {
        let path = root.join(ROUTES_FILE);
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("in {}", path.display()))
    }

    /// The first rule for `node` in `role`, with its place counted from one.
    #[must_use]
    pub fn pick(&self, node: &Node, role: &str) -> Option<(usize, &Rule)> {
        self.rules.iter().enumerate().find(|(_, rule)| rule.fits(node, role)).map(|(index, rule)| (index + 1, rule))
    }

    /// The model of `node` run in `role`: the card's own first, else the first rule that fits,
    /// else `default`.
    #[must_use]
    pub fn choose(&self, node: &Node, role: &str, default: &str) -> Choice {
        if let Some(own) = &node.assignee {
            return Choice { model: own.model.clone(), effort: None, why: Why::Card };
        }
        match self.pick(node, role) {
            Some((position, hit)) => Choice { model: hit.model.clone(), effort: hit.effort, why: Why::Rule(position) },
            None => Choice { model: default.to_owned(), effort: None, why: Why::Default },
        }
    }
}

#[cfg(test)]
mod tests;
