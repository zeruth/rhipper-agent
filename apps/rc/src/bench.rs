//! `rc bench` with no `--project`: the replays of every project the hub knows, a row for each
//! project and one for the pool, so a change that helps one project and hurts another shows.

use std::path::{Path, PathBuf};

/// One line of the bench: how many cases a project (or the pool) replayed and how many passed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) name: String,
    pub(crate) cases: usize,
    pub(crate) passed: usize,
}

impl Row {
    /// The row as the bench prints it.
    pub(crate) fn line(&self) -> String {
        format!("{:<20} {:>4} cases {:>4} passed", self.name, self.cases, self.passed)
    }
}

/// The name of the pooled row.
pub(crate) const POOL: &str = "pool";

/// Replays every project's cases with `replay` (one pass or fail per case) and returns a row
/// for each project, in the order given, then the pooled row.
pub(crate) fn rows(projects: &[PathBuf], replay: impl Fn(&Path) -> Vec<bool>) -> Vec<Row> {
    let mut rows: Vec<Row> = projects
        .iter()
        .map(|root| {
            let outcomes = replay(root);
            Row { name: ra_console::hub::project_name(root), cases: outcomes.len(), passed: outcomes.iter().filter(|ok| **ok).count() }
        })
        .collect();
    let pool = Row { name: POOL.to_owned(), cases: rows.iter().map(|r| r.cases).sum(), passed: rows.iter().map(|r| r.passed).sum() };
    rows.push(pool);
    rows
}

/// The projects a bench without `--project` replays: the hub's, in its priority order.
pub(crate) fn hub_projects() -> Vec<PathBuf> {
    ra_console::hub::known_projects()
}
