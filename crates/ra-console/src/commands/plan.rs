//! `rc plan`: the agent plans its own next cards from the evidence of its runs, and the walk
//! reads the saving each card claimed back a day later. [`gather`] collects every line the
//! planner may cite, the planner proposes ([`crate::plan::propose`]), and each card that stands
//! lands under the milestone with the lines it cites and what it saves, its complexity held to
//! one run under the turn cap; a dry run shows the plan and adds nothing. [`read_back`] runs
//! first in a walk: a planned card done a day ago has its claim judged, the last day against the
//! day before, on the ledger for turns, tokens and cost and on the gotcha journal for a refused
//! command, and a `[saving]` line in `var/retro.log` says held or missed, once per card.

use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use ra_context::TaskContext;
use ra_gotchas::journal::{self, Summary};
use ra_roadmap::model::{Node, NodeKind, TaskState};
use ra_roadmap::store::{Card, Store};
use ra_roadmap::tree;
use ra_usage::Totals;

use crate::agent::{self, Worker};
use crate::plan::evidence::{self, GOTCHAS_LOG, Line};
use crate::plan::propose;
use crate::retro;

/// The most complexity a planned card gets: one run under the turn cap finishes it.
pub const MAX_COMPLEXITY: u8 = 3;

/// A day: the window a saving is read back over, and how long after a card's last run the
/// walk waits before reading it.
pub const DAY: Duration = Duration::from_hours(24);

/// Every evidence line the planner may cite, in the order the brief numbers them: the ledger's
/// costliest cards when the worker has a ledger, the cards run more than once, the look-backs
/// of this project and the machine's, the gotcha journal, and the other projects' digests.
///
/// # Errors
/// When the roadmap or the ledger cannot be read.
pub async fn gather(store: &Store, root: &Path, worker: &Worker) -> anyhow::Result<Vec<Line>> {
    let nodes = store.all().await?;
    let mut lines = Vec::new();
    if let Some(ledger) = worker.ledger() {
        lines.extend(evidence::ledger(ledger).await?);
    }
    lines.extend(evidence::reruns(root, &nodes));
    lines.extend(retro::evidence(root));
    lines.extend(evidence::gotchas(root));
    lines.extend(evidence::digests(root));
    Ok(lines)
}

/// Where the planned cards go: `under` when it names a milestone or a section, else the last
/// open milestone in the file's order, the phase being defined.
///
/// # Errors
/// No such node, one that is no milestone or section, or no open milestone at all.
pub fn target(nodes: &[Node], under: Option<i64>) -> anyhow::Result<&Node> {
    match under {
        Some(id) => {
            let node = nodes.iter().find(|n| n.id == id).with_context(|| format!("no node {id}"))?;
            anyhow::ensure!(
                matches!(node.kind, NodeKind::Milestone | NodeKind::Section),
                "#{id} is a {}; plan under a milestone or a section",
                node.kind
            );
            Ok(node)
        }
        None => nodes
            .iter()
            .rev()
            .find(|n| n.kind == NodeKind::Milestone && !matches!(n.state, TaskState::Accepted | TaskState::Cancelled))
            .context("no open milestone to plan under; add one, or name one with --under"),
    }
}

/// Plans from the evidence and adds the cards under the milestone; with `dry`, shows them and
/// adds nothing. The lines: the head with the planner and its cost, one line per card with what
/// it cites and saves, the dropped ones with their reasons, and the end line with the turn cap
/// each run is budgeted under.
///
/// # Errors
/// No milestone to plan under, no evidence, a failed call, or a refusal ([`propose::Refusal`]).
pub async fn plan(store: &Store, worker: &Worker, under: Option<i64>, dry: bool) -> anyhow::Result<Vec<String>> {
    let root = agent::workspace_of(store);
    let nodes = store.all().await?;
    let target = target(&nodes, under)?;
    let (under, title) = (target.id, target.title.clone());
    let lines = gather(store, &root, worker).await?;
    let proposal = propose::propose(&root, worker, &lines).await?;
    let cost = proposal.usage.cost_usd.map_or_else(|| "cost unreported".to_owned(), |c| format!("${c:.4}"));
    let mut out = vec![format!(
        "{} card(s) from {} evidence line(s) by {} · in {} out {} · {cost}",
        proposal.cards.len(),
        lines.len(),
        agent::short(&proposal.model),
        proposal.usage.prompt_tokens,
        proposal.usage.completion_tokens
    )];
    for dropped in &proposal.dropped {
        out.push(format!("  dropped {dropped}"));
    }
    if dry {
        for card in &proposal.cards {
            out.push(format!(
                "  {} {{c{}}} · evidence: {} · saves: {}",
                card.item.title,
                planned_complexity(&card.item),
                cited(&lines, card),
                card.saves
            ));
        }
        out.push(format!("dry run: nothing added under #{under} {title}"));
        return Ok(out);
    }
    for card in &proposal.cards {
        let sources = cited(&lines, card);
        let complexity = planned_complexity(&card.item);
        let id = store
            .insert_card(
                Some(under),
                NodeKind::Task,
                &card.item.title,
                &Card {
                    objective: card.item.objective.as_deref(),
                    acceptance: card.item.acceptance.as_deref(),
                    complexity,
                    files: &card.item.files,
                    evidence: Some(&sources),
                    saving: Some(&card.saves),
                    ..Card::new()
                },
            )
            .await?;
        // the human clause rides the notes, which the window shows, and stays out of the acceptance
        if let Some(person) = &card.person {
            store.set_notes(id, &format!("person: {person}")).await?;
        }
        out.push(format!("#{id} {} {{c{complexity}}} · evidence: {sources} · saves: {}", card.item.title, card.saves));
    }
    // the cap each run is budgeted under is the project's, named so the person sees the budget
    let cap = crate::router::Router::load(&root).ok().and_then(|router| router.worker.turns());
    let budget = cap.map_or_else(
        || "no turn cap: [worker] turns in rhipper.toml would hold each to one run".to_owned(),
        |turns| format!("one run each under the turn cap of {turns} turn(s)"),
    );
    let notice =
        format!("added {} planned card(s) under #{under} {title} by {} · {budget}", proposal.cards.len(), agent::short(&proposal.model));
    crate::pair::event(&root, &notice);
    out.push(notice);
    Ok(out)
}

/// The sources of the lines a card cites, each once, in the card's order: what its `evidence:`
/// field holds.
fn cited(lines: &[Line], card: &propose::Card) -> String {
    let mut sources: Vec<&str> = Vec::new();
    for source in card.evidence.iter().filter_map(|n| lines.get(n.wrapping_sub(1))).map(|line| line.source.as_str()) {
        if !sources.contains(&source) {
            sources.push(source);
        }
    }
    sources.join(", ")
}

/// The complexity a planned card lands with: the planner's, held to [`MAX_COMPLEXITY`].
fn planned_complexity(item: &ra_roadmap::outline::Item) -> u8 {
    item.complexity.clamp(1, MAX_COMPLEXITY)
}

/// What a claimed saving is measured by, read from its words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Measure {
    /// The turns the day's runs took: a claim of turns, and of a person's intervention, which
    /// the turns a card takes tell best.
    Turns,
    /// The tokens a turn carried.
    Tokens,
    /// The dollars, when the ledger reports them; the tokens a turn otherwise.
    Cost,
    /// The gotcha hits and the refused commands of the journal.
    Refusals,
}

impl Measure {
    /// The measure a saving names: a refused command or a gotcha, tokens or context, cost or
    /// dollars; turns for anything else.
    #[must_use]
    pub fn of(saving: &str) -> Self {
        let words = saving.to_ascii_lowercase();
        if ["refus", "gotcha", "command", "denied"].iter().any(|w| words.contains(w)) {
            Self::Refusals
        } else if words.contains("token") || words.contains("context") {
            Self::Tokens
        } else if words.contains('$') || words.contains("cost") || words.contains("dollar") || words.contains("cheap") {
            Self::Cost
        } else {
            Self::Turns
        }
    }
}

/// One day's figures, as a saving is judged on them.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Day {
    pub turns: u64,
    pub tokens_per_turn: u64,
    pub cost: Option<f64>,
    pub refusals: usize,
}

impl Day {
    /// The day of the ledger's totals and the journal's hits.
    #[must_use]
    pub fn of(totals: &Totals, refusals: usize) -> Self {
        Self { turns: totals.turns, tokens_per_turn: totals.tokens() / totals.turns.max(1), cost: totals.cost_usd, refusals }
    }

    /// The figure of one measure, with its text.
    #[allow(clippy::cast_precision_loss)]
    fn figure(self, measure: Measure) -> (f64, String) {
        match measure {
            Measure::Turns => (self.turns as f64, format!("{} turn(s)", self.turns)),
            Measure::Tokens => (self.tokens_per_turn as f64, format!("{} tokens a turn", self.tokens_per_turn)),
            Measure::Cost => match self.cost {
                Some(cost) => (cost, format!("${cost:.2}")),
                None => self.figure(Measure::Tokens),
            },
            Measure::Refusals => (self.refusals as f64, format!("{} refusal(s)", self.refusals)),
        }
    }
}

/// The verdict on a claim: held when the last day is under the day before on the claim's
/// measure (or both are at none, for refusals), with the two figures; `None` when the day
/// before has nothing to compare, so the claim waits for the next walk.
#[must_use]
pub fn verdict(saving: &str, before: Day, after: Day) -> Option<(bool, String)> {
    let measure = Measure::of(saving);
    let (was, was_text) = before.figure(measure);
    let (is, is_text) = after.figure(measure);
    if was == 0.0 && measure != Measure::Refusals {
        return None;
    }
    let held = is < was || (is == 0.0 && was == 0.0);
    Some((held, format!("claimed \"{saving}\"; {was_text} the day before, {is_text} the last day")))
}

/// Reads the planned cards' claims back: every card with a saving, implemented or accepted,
/// whose last run ended a day or more ago and whose claim has no verdict yet, judged on the
/// last day against the day before; one `[saving]` line each in `var/retro.log`, returned too.
/// Nothing is an empty list: a walk is not stopped by a reading.
pub async fn read_back(store: &Store, root: &Path, worker: &Worker) -> Vec<String> {
    let Ok(nodes) = store.all().await else { return Vec::new() };
    let now = agent::unix_now();
    let due: Vec<&Node> = tree::cards(&nodes)
        .filter(|node| node.saving.is_some() && matches!(node.state, TaskState::Implemented | TaskState::Accepted))
        .filter(|node| !retro::saving_read(root, node.id))
        .filter(|node| TaskContext::open(root, node.id).handoffs.last().is_some_and(|run| run.when + DAY.as_secs() <= now))
        .collect();
    if due.is_empty() {
        return Vec::new();
    }
    let (before, after) = days(root, worker, now).await;
    let mut out = Vec::new();
    for node in due {
        let Some(saving) = node.saving.as_deref() else { continue };
        let Some((held, why)) = verdict(saving, before, after) else { continue };
        match retro::record_saving(root, node.id, held, &why) {
            Ok(line) => out.push(line),
            Err(e) => tracing::warn!("the saving of #{} was not recorded: {e:#}", node.id),
        }
    }
    out
}

/// The day before and the last day: the ledger's totals of each window when the worker has a
/// ledger, and the journal's hits in each.
async fn days(root: &Path, worker: &Worker, now: u64) -> (Day, Day) {
    let (older, newer) = match worker.ledger() {
        Some(ledger) => (
            ledger.totals_between(DAY * 2, DAY).await.unwrap_or_default(),
            ledger.totals_between(DAY, Duration::ZERO).await.unwrap_or_default(),
        ),
        None => (Totals::default(), Totals::default()),
    };
    let journal = root.join(GOTCHAS_LOG);
    let hits = |summary: &Summary| summary.counts.iter().chain(&summary.unmatched).map(|(_, n)| *n).sum::<usize>();
    let last_day = hits(&journal::summary_since(&journal, now.saturating_sub(DAY.as_secs())));
    let two_days = hits(&journal::summary_since(&journal, now.saturating_sub(2 * DAY.as_secs())));
    (Day::of(&older, two_days.saturating_sub(last_day)), Day::of(&newer, last_day))
}
