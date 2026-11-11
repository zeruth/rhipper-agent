//! The per-card reading of the ledger: what a card cost over every run on it, and how many
//! tokens a turn took, as the report tab and `rc status --all` show it beside the card.

use std::time::Duration;

use sqlx::Row;

use crate::{Totals, UsageStore, sums, totals_of};

/// The look-back of the ledger's evidence: the costliest cards of the last day.
pub const LAST_DAY: Duration = Duration::from_hours(24);

/// The cost of one card, summed over every call that served its node.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardCost {
    pub node: i64,
    /// The sums of the card's calls; `calls` is 0 for a card the ledger has not seen.
    pub totals: Totals,
}

impl CardCost {
    /// The prompt and the completion tokens the card spent, over every run.
    #[must_use]
    pub fn tokens(&self) -> u64 {
        self.totals.tokens()
    }

    /// The tokens one turn took on average, 0 before the first turn: the figure that tells a
    /// card that reads its whole tree each turn from one that works from the outline.
    #[must_use]
    pub fn tokens_per_turn(&self) -> u64 {
        self.tokens().checked_div(self.totals.turns).unwrap_or(0)
    }
}

impl UsageStore {
    /// What the card `node` cost: its calls, turns and tokens summed over every run on it,
    /// with the cost `None` when no call reported one.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn card_cost(&self, node: i64) -> anyhow::Result<CardCost> {
        let row = sqlx::query(concat!("select ", sums!(), " from usage where node = ?")).bind(node).fetch_one(&self.pool).await?;
        Ok(CardCost { node, totals: totals_of(&row) })
    }

    /// The cost of every card the ledger has seen, by id; the rows that served no node are
    /// left out.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn card_costs(&self) -> anyhow::Result<Vec<CardCost>> {
        let rows = sqlx::query(concat!("select node, ", sums!(), " from usage where node is not null group by node order by node"))
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(|row| CardCost { node: row.get("node"), totals: totals_of(row) }).collect())
    }

    /// The `limit` costliest cards of the rows written in the last `since`, the costliest first,
    /// each with its turns and tokens summed over those rows: the ledger's evidence of where the
    /// money went in the last day. A card whose rows reported no cost ranks last, by its turns;
    /// the rows that served no node are left out.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn costliest_cards(&self, since: Duration, limit: u32) -> anyhow::Result<Vec<CardCost>> {
        // SQLite does the date arithmetic: `at` is its own `datetime('now')` text
        let window = format!("-{} seconds", since.as_secs());
        let rows = sqlx::query(concat!(
            "select node, ",
            sums!(),
            " from usage where node is not null and at >= datetime('now', ?) \
             group by node order by cost desc nulls last, turns desc, node limit ?"
        ))
        .bind(window)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(|row| CardCost { node: row.get("node"), totals: totals_of(row) }).collect())
    }
}
