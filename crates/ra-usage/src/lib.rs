//! Token and cost accounting. One row per completion: the model, the tokens, the turns, the
//! cost the provider reported and the roadmap node it served. The provider's own dashboard stays
//! the authoritative bill; this is the local record the budgets and the routing read.

use anyhow::Context;
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};

mod query;
pub use query::{CardCost, LAST_DAY};

/// The usage store over one `SQLite` database.
#[derive(Clone, Debug)]
pub struct UsageStore {
    pool: SqlitePool,
}

/// One completion's token counts and cost.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// The model id, as the provider knows it.
    pub model: String,
    /// The role the call played when it was not the worker's own: `summary`, `digest`,
    /// `classify`, `reviewer`, `planner`; `None` for a worker's run, a ping or a chat.
    pub role: Option<String>,
    /// The roadmap node it served, when known.
    pub node: Option<i64>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cached_tokens: u64,
    /// The cost in dollars as the provider reported it; `None` when it reported none.
    pub cost_usd: Option<f64>,
    /// Whether the cost was computed from the model's list prices rather than reported, so a
    /// reader knows which rows the provider's bill will not match to the cent.
    pub cost_computed: bool,
    /// The assistant turns the completion took: one from a bare model, what the harness
    /// reports from a run.
    pub turns: u64,
}

/// The sums over a set of entries.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Totals {
    pub calls: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cached_tokens: u64,
    /// The summed cost of the entries that reported one; `None` when none did.
    pub cost_usd: Option<f64>,
    pub turns: u64,
}

/// One row of the ledger, as the ledger tab lists the calls of a card.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    /// When the row was written, as `SQLite` writes it: `YYYY-MM-DD HH:MM:SS`, UTC.
    pub at: String,
    pub model: String,
    /// The role the call played, as [`Entry::role`] has it.
    pub role: Option<String>,
    pub turns: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cached_tokens: u64,
    pub cost_usd: Option<f64>,
    /// Whether the cost was computed from list prices rather than reported.
    pub cost_computed: bool,
}

/// One row as the dashboard's token gauges read it: when it was written, the tokens in and
/// out together, and the cost.
#[derive(Debug, Clone, PartialEq)]
pub struct Spend {
    /// When the row was written, as `SQLite` writes it: `YYYY-MM-DD HH:MM:SS`, UTC.
    pub at: String,
    /// The prompt and the completion tokens together; the cache is not counted.
    pub tokens: u64,
    pub cost_usd: Option<f64>,
}

impl Totals {
    /// The prompt and the completion tokens together, as the gauges count a run.
    #[must_use]
    pub fn tokens(&self) -> u64 {
        self.prompt_tokens + self.completion_tokens
    }

    /// Adds one entry.
    pub fn add(&mut self, entry: &Entry) {
        self.calls += 1;
        self.prompt_tokens += entry.prompt_tokens;
        self.completion_tokens += entry.completion_tokens;
        self.cached_tokens += entry.cached_tokens;
        self.turns += entry.turns;
        if let Some(cost) = entry.cost_usd {
            self.cost_usd = Some(self.cost_usd.unwrap_or(0.0) + cost);
        }
    }

    /// Adds another set of sums, for a node's subtree.
    pub fn merge(&mut self, other: &Self) {
        self.calls += other.calls;
        self.prompt_tokens += other.prompt_tokens;
        self.completion_tokens += other.completion_tokens;
        self.cached_tokens += other.cached_tokens;
        self.turns += other.turns;
        if let Some(cost) = other.cost_usd {
            self.cost_usd = Some(self.cost_usd.unwrap_or(0.0) + cost);
        }
    }
}

impl UsageStore {
    /// Opens the store at `path`, creating the database and its schema when they are new.
    ///
    /// # Errors
    /// When the file cannot be opened or the schema cannot be written.
    pub async fn open(path: &std::path::Path) -> anyhow::Result<Self> {
        let options = SqliteConnectOptions::new().filename(path).create_if_missing(true);
        let pool = pool(options).await.with_context(|| format!("cannot open the usage store at {}", path.display()))?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    /// Opens the store in memory, for tests.
    ///
    /// # Errors
    /// When the schema cannot be written.
    pub async fn in_memory() -> anyhow::Result<Self> {
        let store = Self { pool: pool(SqliteConnectOptions::new().in_memory(true)).await? };
        store.migrate().await?;
        Ok(store)
    }

    async fn migrate(&self) -> anyhow::Result<()> {
        sqlx::query(
            "create table if not exists usage (
                id integer primary key autoincrement,
                at text not null default (datetime('now')),
                model text not null,
                node integer,
                prompt_tokens integer not null,
                completion_tokens integer not null,
                cached_tokens integer not null,
                cost_usd real,
                turns integer not null default 1,
                cost_computed integer not null default 0,
                role text
            )",
        )
        .execute(&self.pool)
        .await?;
        // a ledger from before the turn count reads as one turn each; one from before the
        // computed mark reads as reported throughout; one from before the role tag reads as
        // the worker's calls throughout
        let columns = sqlx::query("pragma table_info(usage)").fetch_all(&self.pool).await?;
        let has = |name: &str| columns.iter().any(|c| c.get::<String, _>("name") == name);
        if !has("turns") {
            sqlx::query("alter table usage add column turns integer not null default 1").execute(&self.pool).await?;
        }
        if !has("cost_computed") {
            sqlx::query("alter table usage add column cost_computed integer not null default 0").execute(&self.pool).await?;
        }
        if !has("role") {
            sqlx::query("alter table usage add column role text").execute(&self.pool).await?;
        }
        Ok(())
    }

    /// Records one completion.
    ///
    /// # Errors
    /// When the row cannot be written.
    pub async fn record(&self, entry: &Entry) -> anyhow::Result<()> {
        self.record_ago(entry, std::time::Duration::ZERO).await
    }

    /// Records one completion as written `ago` before now: an import of an older ledger, and
    /// the tests of the readings that compare one day with the one before it.
    ///
    /// # Errors
    /// When the row cannot be written.
    pub async fn record_ago(&self, entry: &Entry, ago: std::time::Duration) -> anyhow::Result<()> {
        // SQLite does the date arithmetic, as the column's default does
        let at = format!("-{} seconds", ago.as_secs());
        sqlx::query(
            "insert into usage (at, model, role, node, prompt_tokens, completion_tokens, cached_tokens, cost_usd, turns, cost_computed) \
             values (datetime('now', ?), ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(at)
        .bind(&entry.model)
        .bind(&entry.role)
        .bind(entry.node)
        .bind(i64::try_from(entry.prompt_tokens)?)
        .bind(i64::try_from(entry.completion_tokens)?)
        .bind(i64::try_from(entry.cached_tokens)?)
        .bind(entry.cost_usd)
        .bind(i64::try_from(entry.turns)?)
        .bind(entry.cost_computed)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The totals over every row.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn totals(&self) -> anyhow::Result<Totals> {
        let row = sqlx::query(concat!("select ", sums!(), " from usage")).fetch_one(&self.pool).await?;
        Ok(totals_of(&row))
    }

    /// The totals over the rows written between `older` and `newer` before now, the older
    /// bound out and the newer in, so a `newer` of zero takes a row written this second: one
    /// day against the day before it, as a saving is read back.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn totals_between(&self, older: std::time::Duration, newer: std::time::Duration) -> anyhow::Result<Totals> {
        let row = sqlx::query(concat!("select ", sums!(), " from usage where at > datetime('now', ?) and at <= datetime('now', ?)"))
            .bind(format!("-{} seconds", older.as_secs()))
            .bind(format!("-{} seconds", newer.as_secs()))
            .fetch_one(&self.pool)
            .await?;
        Ok(totals_of(&row))
    }

    /// The id of the newest row, 0 when there is none: a reader's mark of where its session
    /// starts, so `totals_after` and `recent` read only the rows since.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn last_id(&self) -> anyhow::Result<i64> {
        let row = sqlx::query("select coalesce(max(id), 0) as id from usage").fetch_one(&self.pool).await?;
        Ok(row.get("id"))
    }

    /// The totals over the rows written after the row `after`; 0 sums every row.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn totals_after(&self, after: i64) -> anyhow::Result<Totals> {
        let row = sqlx::query(concat!("select ", sums!(), " from usage where id > ?")).bind(after).fetch_one(&self.pool).await?;
        Ok(totals_of(&row))
    }

    /// The totals over the rows up to and including the row `until`: what the ledger held
    /// before a run, as its saving is judged against it.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn totals_until(&self, until: i64) -> anyhow::Result<Totals> {
        let row = sqlx::query(concat!("select ", sums!(), " from usage where id <= ?")).bind(until).fetch_one(&self.pool).await?;
        Ok(totals_of(&row))
    }

    /// The newest `limit` rows written after the row `after`, the oldest first, with the tokens
    /// and the cost of each; 0 reads from the start.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn recent(&self, after: i64, limit: u32) -> anyhow::Result<Vec<Spend>> {
        let rows = sqlx::query("select at, prompt_tokens, completion_tokens, cost_usd from usage where id > ? order by id desc limit ?")
            .bind(after)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        let mut spends: Vec<Spend> = rows
            .iter()
            .map(|row| {
                let count = |name: &str| u64::try_from(row.get::<i64, _>(name)).unwrap_or(0);
                Spend { at: row.get("at"), tokens: count("prompt_tokens") + count("completion_tokens"), cost_usd: row.get("cost_usd") }
            })
            .collect();
        spends.reverse();
        Ok(spends)
    }

    /// The totals of every model that has rows, by model id, the costliest first.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn by_model(&self) -> anyhow::Result<Vec<(String, Totals)>> {
        let rows = sqlx::query(concat!("select model, ", sums!(), " from usage group by model order by cost desc, calls desc"))
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(|row| (row.get("model"), totals_of(row))).collect())
    }

    /// The totals of every model and role that has rows, the costliest first: a cheap role's
    /// calls listed beside the worker's, whose role is `None`.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn by_role(&self) -> anyhow::Result<Vec<(String, Option<String>, Totals)>> {
        let rows = sqlx::query(concat!("select model, role, ", sums!(), " from usage group by model, role order by cost desc, calls desc"))
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(|row| (row.get("model"), row.get::<Option<String>, _>("role"), totals_of(row))).collect())
    }

    /// The totals of every roadmap node that has rows, by id, with `None` for the rows that
    /// served no node (a ping, a chat); a reader sums them up the tree.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn by_node(&self) -> anyhow::Result<Vec<(Option<i64>, Totals)>> {
        let rows = sqlx::query(concat!("select node, ", sums!(), " from usage group by node order by node")).fetch_all(&self.pool).await?;
        Ok(rows.iter().map(|row| (row.get::<Option<i64>, _>("node"), totals_of(row))).collect())
    }

    /// The calls that served one node, the latest first; `None` lists the calls that served
    /// no node.
    ///
    /// # Errors
    /// When the query fails.
    pub async fn calls_of(&self, node: Option<i64>) -> anyhow::Result<Vec<Call>> {
        let rows = sqlx::query(
            "select at, model, role, turns, prompt_tokens, completion_tokens, cached_tokens, cost_usd, cost_computed from usage \
             where node is ? order by id desc",
        )
        .bind(node)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|row| {
                let count = |name: &str| u64::try_from(row.get::<i64, _>(name)).unwrap_or(0);
                Call {
                    at: row.get("at"),
                    model: row.get("model"),
                    role: row.get("role"),
                    turns: count("turns"),
                    prompt_tokens: count("prompt_tokens"),
                    completion_tokens: count("completion_tokens"),
                    cached_tokens: count("cached_tokens"),
                    cost_usd: row.get("cost_usd"),
                    cost_computed: row.get("cost_computed"),
                }
            })
            .collect())
    }
}

/// One connection, so the writes of parallel jobs queue instead of colliding, and so an in-memory
/// database is one database and not one per connection.
async fn pool(options: SqliteConnectOptions) -> Result<SqlitePool, sqlx::Error> {
    SqlitePoolOptions::new().max_connections(1).connect_with(options).await
}

/// The aggregate columns of a totals query, as a literal so the SQL stays `'static`.
macro_rules! sums {
    () => {
        "count(*) as calls, \
         coalesce(sum(prompt_tokens), 0) as prompt, \
         coalesce(sum(completion_tokens), 0) as completion, \
         coalesce(sum(cached_tokens), 0) as cached, \
         sum(cost_usd) as cost, \
         coalesce(sum(turns), 0) as turns"
    };
}
use sums;

fn totals_of(row: &sqlx::sqlite::SqliteRow) -> Totals {
    let count = |name: &str| u64::try_from(row.get::<i64, _>(name)).unwrap_or(0);
    Totals {
        calls: count("calls"),
        prompt_tokens: count("prompt"),
        completion_tokens: count("completion"),
        cached_tokens: count("cached"),
        cost_usd: row.get::<Option<f64>, _>("cost"),
        turns: count("turns"),
    }
}

#[cfg(test)]
mod tests;
