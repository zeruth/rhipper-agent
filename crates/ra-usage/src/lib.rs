//! Token and cost accounting. One row per execution: the model, the tokens, the estimated cost
//! and the task it served. OpenRouter's activity dashboard stays the authoritative source; this
//! is the local estimate the budgets and the routing read.

use anyhow::Context;
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};

/// The usage store over one SQLite database.
#[derive(Clone)]
pub struct UsageStore {
    pool: SqlitePool,
}

/// One execution's token counts and cost.
#[derive(Debug, Clone)]
pub struct Entry {
    /// The OpenRouter model id.
    pub model: String,
    /// The roadmap node it served, when known.
    pub node: Option<i64>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cached_tokens: u64,
    /// The estimated cost in dollars.
    pub cost_usd: f64,
}

impl UsageStore {
    /// Opens the store at `path`, creating the schema when the database is new.
    pub async fn open(path: &std::path::Path) -> anyhow::Result<Self> {
        let options = SqliteConnectOptions::new().filename(path).create_if_missing(true);
        let pool = SqlitePool::connect_with(options).await.with_context(|| format!("cannot open the usage store at {}", path.display()))?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    /// Opens the store in memory, for tests.
    pub async fn in_memory() -> anyhow::Result<Self> {
        let pool = SqlitePool::connect_with(SqliteConnectOptions::new().in_memory(true)).await?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    async fn migrate(&self) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            create table if not exists usage (
                id integer primary key autoincrement,
                at text not null default (datetime('now')),
                model text not null,
                node integer,
                prompt_tokens integer not null,
                completion_tokens integer not null,
                cached_tokens integer not null,
                cost_usd real not null
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Records one execution.
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

    /// The summed cost and tokens of one model.
    pub async fn totals_by_model(&self, model: &str) -> anyhow::Result<(f64, u64)> {
        let row = sqlx::query("select coalesce(sum(cost_usd), 0) as cost, coalesce(sum(prompt_tokens + completion_tokens), 0) as tokens from usage where model = ?")
            .bind(model)
            .fetch_one(&self.pool)
            .await?;
        Ok((row.get("cost"), u64::try_from(row.get::<i64, _>("tokens")).unwrap_or(0)))
    }
}
