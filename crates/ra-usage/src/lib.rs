//! Token and cost accounting. One row per completion: the model, the tokens, the cost the
//! provider reported and the roadmap node it served. The provider's own dashboard stays the
//! authoritative bill; this is the local record the budgets and the routing read.

use anyhow::Context;
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};

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
    /// The roadmap node it served, when known.
    pub node: Option<i64>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cached_tokens: u64,
    /// The cost in dollars as the provider reported it; `None` when it reported none.
    pub cost_usd: Option<f64>,
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
}

impl Totals {
    /// Adds one entry.
    pub fn add(&mut self, entry: &Entry) {
        self.calls += 1;
        self.prompt_tokens += entry.prompt_tokens;
        self.completion_tokens += entry.completion_tokens;
        self.cached_tokens += entry.cached_tokens;
        if let Some(cost) = entry.cost_usd {
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
                cost_usd real
            )",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Records one completion.
    ///
    /// # Errors
    /// When the row cannot be written.
    pub async fn record(&self, entry: &Entry) -> anyhow::Result<()> {
        sqlx::query("insert into usage (model, node, prompt_tokens, completion_tokens, cached_tokens, cost_usd) values (?, ?, ?, ?, ?, ?)")
            .bind(&entry.model)
            .bind(entry.node)
            .bind(i64::try_from(entry.prompt_tokens)?)
            .bind(i64::try_from(entry.completion_tokens)?)
            .bind(i64::try_from(entry.cached_tokens)?)
            .bind(entry.cost_usd)
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
         sum(cost_usd) as cost"
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
    }
}

#[cfg(test)]
mod tests;
