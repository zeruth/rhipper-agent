//! The card lines under a project's row of `rc status --all`: the cards a run has in hand, each
//! with what it cost and the tokens a turn took, from the project's ledger; blank for a card no
//! call has served.

use std::path::Path;

use ra_roadmap::model::{Node, NodeKind, TaskState};
use ra_usage::{CardCost, UsageStore};

/// Whether a card is in hand: taken up by a run and not yet accepted or put down, so the
/// status names it. A ready card is counted by the project's row, an accepted one is history.
fn in_hand(state: TaskState) -> bool {
    matches!(
        state,
        TaskState::Assigned
            | TaskState::InProgress
            | TaskState::Implemented
            | TaskState::Testing
            | TaskState::Review
            | TaskState::NeedsRevision
            | TaskState::Failed
    )
}

/// The lines of the cards in hand of the project at `root`, in the roadmap's order; none
/// without a roadmap to read. A project without a ledger gets none opened for it: the open
/// would create the file.
pub(crate) async fn card_lines_of(root: &Path) -> Vec<String> {
    let Some(nodes) = std::fs::read_to_string(root.join("rhipper.md")).ok().and_then(|text| ra_roadmap::markdown::parse(&text).ok()) else {
        return Vec::new();
    };
    let ledger = root.join("var/usage.db");
    let costs = if ledger.is_file() {
        match UsageStore::open(&ledger).await {
            Ok(ledger) => ledger.card_costs().await.unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    };
    card_lines(&nodes, &costs)
}

/// The line of every card in hand among `nodes`, each with its cost from `costs`.
#[must_use]
pub(crate) fn card_lines(nodes: &[Node], costs: &[CardCost]) -> Vec<String> {
    nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Task && in_hand(n.state))
        .map(|n| card_line(n, costs.iter().find(|c| c.node == n.id)))
        .collect()
}

/// One card's line, indented under its project's row: the id, the title and the state, then
/// the dollars and the average tokens a turn when the ledger has a call for it.
#[must_use]
pub(crate) fn card_line(node: &Node, cost: Option<&CardCost>) -> String {
    let head = format!("  #{} {} [{}]", node.id, node.title, node.state);
    match cost.filter(|c| c.totals.calls > 0) {
        None => head,
        Some(cost) => {
            let dollars = cost.totals.cost_usd.map_or_else(|| String::from("$—"), |c| format!("${c:.2}"));
            format!("{head}  {dollars} · {} tok/turn", compact(cost.tokens_per_turn()))
        }
    }
}

/// A token count that reads at a glance: `950`, `12.3k`, `1.2M`.
pub(crate) fn compact(count: u64) -> String {
    match count {
        0..=999 => count.to_string(),
        1_000..=999_999 => format!("{}.{}k", count / 1_000, count % 1_000 / 100),
        _ => format!("{}.{}M", count / 1_000_000, count % 1_000_000 / 100_000),
    }
}
