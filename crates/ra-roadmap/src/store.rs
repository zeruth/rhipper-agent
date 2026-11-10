//! The SQLite store of the roadmap. It creates its schema and reads and writes the nodes.

use std::path::Path;

use anyhow::Context;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions, SqliteRow};
use sqlx::{Row, SqliteConnection};

use crate::model::{Assignment, Node, NodeKind, TaskState};

/// The store over one SQLite database.
#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    /// Opens the store at `path`, creating the parent directory and the schema when new.
    pub async fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| format!("cannot create {}", parent.display()))?;
        }
        let options = SqliteConnectOptions::new().filename(path).create_if_missing(true).foreign_keys(true);
        let pool =
            SqlitePool::connect_with(options).await.with_context(|| format!("cannot open the roadmap store at {}", path.display()))?;
        Self::migrate(&mut *pool.acquire().await?).await?;
        Ok(Self { pool })
    }

    /// Opens the store in memory, for tests. The pool holds one connection: each connection of an
    /// in-memory SQLite pool would otherwise see its own empty database.
    pub async fn in_memory() -> anyhow::Result<Self> {
        let options = SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
        let pool = SqlitePoolOptions::new().max_connections(1).connect_with(options).await?;
        Self::migrate(&mut *pool.acquire().await?).await?;
        Ok(Self { pool })
    }

    async fn migrate(connection: &mut SqliteConnection) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            create table if not exists nodes (
                id integer primary key autoincrement,
                parent integer references nodes(id),
                kind text not null,
                title text not null,
                objective text,
                acceptance text,
                depends_on text not null default '[]',
                state text not null default 'defined',
                assignee_model text,
                assignee_role text,
                complexity integer not null default 3,
                notes text
            )
            "#,
        )
        .execute(connection)
        .await?;
        Ok(())
    }

    /// Inserts a node under `parent` (or as a project when it is `None`) and returns its id.
    pub async fn insert(
        &self,
        parent: Option<i64>,
        kind: NodeKind,
        title: &str,
        objective: Option<&str>,
        acceptance: Option<&str>,
        complexity: u8,
    ) -> anyhow::Result<i64> {
        let id = sqlx::query("insert into nodes (parent, kind, title, objective, acceptance, complexity) values (?, ?, ?, ?, ?, ?)")
            .bind(parent)
            .bind(kind.serde_name())
            .bind(title)
            .bind(objective)
            .bind(acceptance)
            .bind(i64::from(complexity))
            .execute(&self.pool)
            .await?
            .last_insert_rowid();
        Ok(id)
    }

    /// Adds a dependency: `id` waits for `waits_for`.
    pub async fn depend_on(&self, id: i64, waits_for: i64) -> anyhow::Result<()> {
        let mut deps = self.read(id).await?.depends_on;
        if !deps.contains(&waits_for) {
            deps.push(waits_for);
            sqlx::query("update nodes set depends_on = ? where id = ?").bind(serde_json_deps(&deps)?).bind(id).execute(&self.pool).await?;
        }
        Ok(())
    }

    /// Reads one node.
    pub async fn read(&self, id: i64) -> anyhow::Result<Node> {
        sqlx::query("select * from nodes where id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map(|row| node_from_row(&row))
            .context("no node with that id")
    }

    /// Reads every node, ordered so that a parent comes before its children.
    pub async fn all(&self) -> anyhow::Result<Vec<Node>> {
        let rows = sqlx::query("select * from nodes order by id").fetch_all(&self.pool).await?;
        Ok(rows.iter().map(|r| node_from_row(r)).collect())
    }

    /// Sets the state of a node.
    pub async fn set_state(&self, id: i64, state: TaskState) -> anyhow::Result<()> {
        sqlx::query("update nodes set state = ? where id = ?").bind(state.serde_name()).bind(id).execute(&self.pool).await?;
        Ok(())
    }

    /// Assigns a node to a worker, or clears the assignment with `None`.
    pub async fn assign(&self, id: i64, assignment: Option<&Assignment>) -> anyhow::Result<()> {
        sqlx::query("update nodes set assignee_model = ?, assignee_role = ? where id = ?")
            .bind(assignment.map(|a| a.model.as_str()))
            .bind(assignment.map(|a| a.role.as_str()))
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Records the notes of the last execution on a node.
    pub async fn set_notes(&self, id: i64, notes: &str) -> anyhow::Result<()> {
        sqlx::query("update nodes set notes = ? where id = ?").bind(notes).bind(id).execute(&self.pool).await?;
        Ok(())
    }
}

fn serde_json_deps(deps: &[i64]) -> anyhow::Result<String> {
    Ok(serde_json::to_string(deps)?)
}

fn kind_of(name: &str) -> NodeKind {
    match name {
        "milestone" => NodeKind::Milestone,
        "section" => NodeKind::Section,
        "task" => NodeKind::Task,
        "subtask" => NodeKind::Subtask,
        _ => NodeKind::Project,
    }
}

fn state_of(name: &str) -> TaskState {
    match name {
        "ready" => TaskState::Ready,
        "assigned" => TaskState::Assigned,
        "in_progress" => TaskState::InProgress,
        "implemented" => TaskState::Implemented,
        "testing" => TaskState::Testing,
        "review" => TaskState::Review,
        "accepted" => TaskState::Accepted,
        "blocked" => TaskState::Blocked,
        "failed" => TaskState::Failed,
        "needs_revision" => TaskState::NeedsRevision,
        "cancelled" => TaskState::Cancelled,
        _ => TaskState::Defined,
    }
}

fn node_from_row(row: &SqliteRow) -> Node {
    let deps: String = row.get("depends_on");
    Node {
        id: row.get("id"),
        parent: row.get("parent"),
        kind: kind_of(&row.get::<String, _>("kind")),
        title: row.get("title"),
        objective: row.get("objective"),
        acceptance: row.get("acceptance"),
        depends_on: serde_json::from_str(&deps).unwrap_or_default(),
        state: state_of(&row.get::<String, _>("state")),
        assignee: match (row.get::<Option<String>, _>("assignee_model"), row.get::<Option<String>, _>("assignee_role")) {
            (Some(model), Some(role)) => Some(Assignment { model, role }),
            _ => None,
        },
        complexity: u8::try_from(row.get::<i64, _>("complexity")).unwrap_or(3),
        notes: row.get("notes"),
    }
}
