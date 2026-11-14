//! The evidence lines the planner reads: one line per finding, each with the source it is cited
//! from, so a planned card can name the line it comes from and a reader can check it. This file
//! reads the ledger: the costliest cards of the last day, their turns and their average tokens a
//! turn, the figure that tells a card that reads its whole tree each turn from one that works
//! from the outline. It reads the reruns: the cards whose context holds more than one handoff,
//! with how each run ended and the last verdict, the cards a first run did not finish. And it
//! reads the hub: the other projects' digests, each one's headline, what that project is doing
//! or last did. And it reads the gotcha journal: the last day's hits by gotcha and the refused
//! commands no gotcha names, each with its tally. The look-backs of `var/retro.log` and the
//! hub's `notes.log` are read by [`crate::retro::evidence`], which owns their format, each
//! cited by file and line.

use std::path::Path;

use ra_context::{Handoff, TaskContext};
use ra_gotchas::journal::{self, Summary};
use ra_roadmap::model::Node;
use ra_usage::{Call, CardCost, LAST_DAY, UsageStore};

use crate::hub::{self, Digest};

/// How many of the day's costliest cards the ledger contributes.
pub const LEDGER_CARDS: u32 = 5;

/// One line of evidence: what was found and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// Where the line was read, short enough to name in a card: `usage.db #12`, `retro.log:7`.
    pub source: String,
    pub text: String,
}

impl std::fmt::Display for Line {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.source, self.text)
    }
}

/// The ledger's lines: the costliest cards of the last day, the costliest first.
///
/// Each line ends with the card's runs, as the ledger and the handoffs under `root` have them.
///
/// # Errors
/// When the ledger cannot be read.
pub async fn ledger(store: &UsageStore, root: &Path) -> anyhow::Result<Vec<Line>> {
    let mut lines = Vec::new();
    for card in store.costliest_cards(LAST_DAY, LEDGER_CARDS).await? {
        let calls = store.calls_of(Some(card.node)).await?;
        let handoffs = TaskContext::open(root, card.node).handoffs;
        lines.push(ledger_line_by_run(&card, &calls, &handoffs));
    }
    Ok(lines)
}

/// [`ledger_line`] with one entry per run after the totals: `r3 22t 51k needs-revision`, the
/// run's turns, its tokens a turn and how it ended. It tells a card that is one long run from
/// one that is many short reruns. `calls` are the card's ledger rows, the latest first; the
/// runs are the worker's own calls, matched in order to `handoffs`.
#[must_use]
pub fn ledger_line_by_run(card: &CardCost, calls: &[Call], handoffs: &[Handoff]) -> Line {
    let mut line = ledger_line(card);
    let runs: Vec<String> = calls
        .iter()
        .rev()
        .filter(|call| call.role.is_none())
        .enumerate()
        .map(|(i, call)| {
            let tokens = (call.prompt_tokens + call.completion_tokens).checked_div(call.turns).unwrap_or(0);
            let outcome = handoffs.get(i).map_or("unfinished", |h| h.outcome.split(" (").next().unwrap_or_default());
            format!("r{} {}t {} {}", i + 1, call.turns, tokens_short(tokens), outcome.replace(' ', "-"))
        })
        .collect();
    if !runs.is_empty() {
        line.text = format!("{}; runs: {}", line.text, runs.join(", "));
    }
    line
}

/// A token count as the run entries show it: whole under a thousand, else in thousands.
fn tokens_short(tokens: u64) -> String {
    if tokens < 1000 { tokens.to_string() } else { format!("{}k", (tokens + 500) / 1000) }
}

/// The line of one card's cost: the card, what it cost, its turns and its average tokens a turn.
#[must_use]
pub fn ledger_line(card: &CardCost) -> Line {
    let cost = card.totals.cost_usd.map_or_else(|| "an unreported amount".to_owned(), |c| format!("${c:.2}"));
    let runs = if card.totals.calls == 1 { "1 run".to_owned() } else { format!("{} runs", card.totals.calls) };
    Line {
        source: format!("usage.db #{}", card.node),
        text: format!(
            "card #{} cost {cost} in the last day over {runs} and {} turn(s), {} tokens a turn on average",
            card.node,
            card.totals.turns,
            card.tokens_per_turn()
        ),
    }
}

/// The reruns' lines: every card of the roadmap whose context under `root` holds more than one
/// handoff, in the roadmap's order.
#[must_use]
pub fn reruns(root: &Path, nodes: &[Node]) -> Vec<Line> {
    reruns_from(nodes, |id| TaskContext::open(root, id).handoffs)
}

/// [`reruns`] with the handoffs of a card read by `runs_of`, for the tests.
pub fn reruns_from(nodes: &[Node], runs_of: impl Fn(i64) -> Vec<Handoff>) -> Vec<Line> {
    ra_roadmap::tree::cards(nodes)
        .filter_map(|node| {
            let runs = runs_of(node.id);
            (runs.len() > 1).then(|| rerun_line(node, &runs))
        })
        .collect()
}

/// The line of one card run more than once: its runs as each ended, and the last verdict when
/// a reviewer gave one.
#[must_use]
pub fn rerun_line(node: &Node, runs: &[Handoff]) -> Line {
    let outcomes: Vec<&str> = runs.iter().map(|run| run.outcome.as_str()).collect();
    let last = runs.last();
    let verdict = match last.and_then(|run| run.verdict.as_deref()) {
        Some(verdict) => verdict.to_owned(),
        None => match last.and_then(|run| last_reason(&run.report)) {
            Some(reason) => format!("no verdict yet, last reason: {reason}"),
            None => "no verdict yet".to_owned(),
        },
    };
    Line {
        source: format!("context #{}", node.id),
        text: format!("card #{} {} ran {} times ({}), last verdict: {verdict}", node.id, node.title, runs.len(), outcomes.join(", ")),
    }
}

/// The first 120 characters of the reason a report gives under its `## Blocked` heading: the
/// rest of the heading's line, else the first line under it.
fn last_reason(report: &str) -> Option<String> {
    let mut lines = report.lines().map(str::trim);
    let heading = lines.find(|line| line.strip_prefix("## Blocked").is_some_and(|rest| !rest.starts_with(char::is_alphanumeric)))?;
    let own = heading.trim_start_matches("## Blocked").trim_start_matches([':', ' ', '-', '—', '·']).trim();
    let reason = if own.is_empty() { lines.find(|line| !line.is_empty())? } else { own };
    Some(reason.chars().take(120).collect())
}

/// The hub's lines: the digest of every other project, its headline as one line, by name.
#[must_use]
pub fn digests(root: &Path) -> Vec<Line> {
    let this = hub::project_name(root);
    hub::digests().iter().filter(|digest| digest.name != this).map(digest_line).collect()
}

/// [`digests`] from the directory named, with the liveness check and the time, for the tests.
pub fn digests_from(dir: &Path, root: &Path, alive: impl Fn(u32) -> bool, now: u64) -> Vec<Line> {
    let this = hub::project_name(root);
    hub::digests_in(dir, alive, now).iter().filter(|digest| digest.name != this).map(digest_line).collect()
}

/// The line of one project's digest: its live run when one is going, else the last card that
/// ended, else only that it is there with its ready count.
#[must_use]
pub fn digest_line(digest: &Digest) -> Line {
    let headline = match (&digest.live, &digest.last) {
        (Some(live), _) => format!("live: {live}"),
        (None, Some(last)) => format!("last: {last}"),
        (None, None) => format!("{} ready, nothing run yet", digest.ready),
    };
    Line { source: format!("hub digest/{}.txt", digest.name), text: format!("{} ({}) · {headline}", digest.name, digest.path) }
}

/// The journal's path under a project root.
pub const GOTCHAS_LOG: &str = "var/gotchas.log";

/// The gotcha journal's lines: the last day's hits by gotcha, the most often first, then the
/// refused commands no gotcha names with their tallies. The routine ones, a build put on a
/// slot, are left out: they are how every build runs, not friction to plan against.
#[must_use]
pub fn gotchas(root: &Path) -> Vec<Line> {
    let mut summary = journal::summary(&root.join(GOTCHAS_LOG));
    let catalogue = std::fs::read_to_string(root.join("var/gotchas.toml"))
        .ok()
        .and_then(|text| ra_gotchas::Catalogue::parse(&text).ok())
        .unwrap_or_else(ra_gotchas::Catalogue::seed);
    summary.counts.retain(|(id, _)| !catalogue.is_routine(id));
    gotcha_lines(&summary)
}

/// [`gotchas`] from a summary already read, for the tests.
#[must_use]
pub fn gotcha_lines(summary: &Summary) -> Vec<Line> {
    let sources = summary
        .counts
        .iter()
        .map(|(id, _)| format!("gotchas.log {id}"))
        .chain(summary.unmatched.iter().map(|_| format!("gotchas.log {}", ra_gotchas::UNMATCHED)));
    sources.zip(summary.lines()).map(|(source, text)| Line { source, text }).collect()
}

#[cfg(test)]
mod tests;
