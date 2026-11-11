//! The planner's call and its verdict on the cards: the brief goes to the planning route, the
//! outline that comes back is read card by card, and a card that cites no evidence line, names
//! no saving or names no check is dropped with its reason. Fewer than [`MIN_CARDS`] or more than
//! [`MAX_CARDS`] cards left is a [`Refusal`], not a plan: the planner either found too little in
//! the evidence or did not pick.

use std::fmt;
use std::path::Path;

use ra_providers::{CompletionRequest, Message, Usage};
use ra_roadmap::model::NodeKind;
use ra_roadmap::outline::{self, Item};

use super::evidence::Line;
use super::prompt::{self, MAX_CARDS, MIN_CARDS};
use crate::agent::{self, Worker};

/// One card that stands: the item the store imports, the evidence numbers it cites and what it
/// says it saves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub item: Item,
    /// The numbers of the brief's lines, from 1.
    pub evidence: Vec<usize>,
    pub saves: String,
}

/// One card the verdict dropped, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dropped {
    pub title: String,
    pub reason: String,
}

impl fmt::Display for Dropped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.title, self.reason)
    }
}

/// The plan: the cards that stand, the ones dropped, and what the call cost.
#[derive(Debug, Clone)]
pub struct Proposal {
    /// The planner's model.
    pub model: String,
    pub usage: Usage,
    pub cards: Vec<Card>,
    pub dropped: Vec<Dropped>,
}

/// Why an answer is no plan: how many cards stood, and the ones dropped on the way.
#[derive(Debug, Clone)]
pub struct Refusal {
    pub kept: usize,
    pub dropped: Vec<Dropped>,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the planner's {} card(s) are no plan: {MIN_CARDS} to {MAX_CARDS} must stand", self.kept)?;
        if !self.dropped.is_empty() {
            let reasons: Vec<String> = self.dropped.iter().map(ToString::to_string).collect();
            write!(f, "; dropped {}", reasons.join("; "))?;
        }
        Ok(())
    }
}

impl std::error::Error for Refusal {}

/// Sends the numbered evidence to the planning route and judges the cards it answers with. The
/// planner's model is the `planner` rule of `rhipper.toml` that names no kind, else the worker's
/// own; the call is booked under it as the planner's.
///
/// # Errors
/// No evidence to plan from, an unreadable `rhipper.toml`, no provider for the model, a failed
/// call, or a [`Refusal`] when fewer than [`MIN_CARDS`] or more than [`MAX_CARDS`] cards stand.
pub async fn propose(root: &Path, worker: &Worker, lines: &[Line]) -> anyhow::Result<Proposal> {
    anyhow::ensure!(!lines.is_empty(), "no evidence to plan from");
    let planner = crate::router::Router::load(root)?;
    let model = planner
        .rules
        .iter()
        .find(|rule| rule.role.eq_ignore_ascii_case(crate::router::PLANNER) && rule.kind.is_none())
        .map_or_else(|| worker.model.clone(), |rule| agent::resolve(&rule.model));
    let provider = worker.provider_for(&model)?;
    tracing::info!(target: crate::trace::PROGRESS, "plan from {} evidence line(s) by {}", lines.len(), agent::short(&model));
    let answer = provider
        .complete(CompletionRequest {
            model: model.clone(),
            messages: vec![Message::system(prompt::INSTRUCTION), Message::user(prompt::brief(lines))],
            temperature: Some(0.2),
            max_tokens: Some(4000),
            tools: Vec::new(),
            workspace: None,
            budget_usd: None,
            resume: None,
            max_turns: Some(1),
            max_context: None,
            effort: Some(ra_roadmap::model::Effort::default().to_string()),
        })
        .await
        .map_err(agent::map_error)?;
    worker.record_as(&model, Some("planner"), None, &answer.usage, answer.turns).await;
    let (cards, dropped) = judge(&answer.text, lines.len());
    if !(MIN_CARDS..=MAX_CARDS).contains(&cards.len()) {
        return Err(Refusal { kept: cards.len(), dropped }.into());
    }
    Ok(Proposal { model, usage: answer.usage, cards, dropped })
}

/// The verdict on an answer: every `- Title` block read as one card, the ones that stand and the
/// ones dropped with their reasons, in the answer's order. `evidence` is how many lines the
/// brief numbered; a card that cites a number past it cites nothing.
#[must_use]
pub fn judge(text: &str, evidence: usize) -> (Vec<Card>, Vec<Dropped>) {
    let mut blocks: Vec<Vec<&str>> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim_end();
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with("```") || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len() - trimmed.len();
        if indent < 2 && (trimmed.starts_with("- ") || trimmed.starts_with("* ")) {
            blocks.push(vec![line]);
        } else if let Some(block) = blocks.last_mut() {
            block.push(line);
        }
    }
    let mut cards = Vec::new();
    let mut dropped = Vec::new();
    for block in blocks {
        match card_of(&block, evidence) {
            Ok(card) => cards.push(card),
            Err(drop) => dropped.push(drop),
        }
    }
    (cards, dropped)
}

/// One block as a card, or the reasons it is none, all of them in one line.
fn card_of(block: &[&str], evidence: usize) -> Result<Card, Dropped> {
    let mut cited = Vec::new();
    let mut saves = String::new();
    let mut rest = String::new();
    for line in block {
        match line.trim().split_once(':') {
            Some((field, value)) if field.trim().eq_ignore_ascii_case("evidence") => {
                cited.extend(value.split(|c: char| !c.is_ascii_digit()).filter_map(|n| n.parse::<usize>().ok()));
            }
            Some((field, value)) if field.trim().eq_ignore_ascii_case("saves") => value.trim().clone_into(&mut saves),
            _ => {
                rest.push_str(line);
                rest.push('\n');
            }
        }
    }
    let mut item = match outline::parse(&rest).map(|outline| outline.items.into_iter().next()) {
        Ok(Some(item)) => item,
        Ok(None) => return Err(Dropped { title: block[0].trim().to_owned(), reason: "is no card line".to_owned() }),
        Err(e) => return Err(Dropped { title: block[0].trim().to_owned(), reason: format!("does not parse: {e:#}") }),
    };
    item.kind = NodeKind::Task;
    let mut reasons = Vec::new();
    if cited.is_empty() {
        reasons.push("cites no evidence line".to_owned());
    } else if let Some(past) = cited.iter().find(|n| **n == 0 || **n > evidence) {
        reasons.push(format!("cites evidence #{past}, the brief has {evidence} line(s)"));
    }
    if saves.is_empty() {
        reasons.push("names no saving".to_owned());
    }
    if !item.acceptance.as_deref().is_some_and(has_check) {
        reasons.push("names no check in backticks".to_owned());
    }
    if reasons.is_empty() {
        Ok(Card { item, evidence: cited, saves })
    } else {
        Err(Dropped { title: item.title, reason: reasons.join(", ") })
    }
}

/// Whether an acceptance names a command: a pair of backticks with something in between.
fn has_check(acceptance: &str) -> bool {
    acceptance.split('`').skip(1).step_by(2).any(|command| !command.trim().is_empty())
}
