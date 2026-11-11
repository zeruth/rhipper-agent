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
}

impl WorkerConfig {
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

/// The rules of a project, in the order of the file, and its worker table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Router {
    #[serde(default, rename = "route")]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub worker: WorkerConfig,
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
