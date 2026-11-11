//! The journal of hits: one line per hit in a file, `time<TAB>node<TAB>id<TAB>where<TAB>excerpt`.
//! `rc gotchas` reads it to say which gotchas keep coming back, the next run reads it for its
//! machine notes, and the planner reads [`summary`]: the last day's hits counted by gotcha, and
//! the refused commands no gotcha names tallied by command, each as a line it can cite.

use std::collections::BTreeMap;
use std::path::Path;

use crate::{Hit, UNMATCHED, Where};

/// The window of [`summary`], in seconds.
pub const LAST_DAY: u64 = 24 * 60 * 60;

/// The journal, at its file.
pub struct Journal<'a> {
    path: &'a Path,
}

/// One line of the journal.
struct Row<'a> {
    time: u64,
    id: &'a str,
    excerpt: &'a str,
}

fn row(line: &str) -> Option<Row<'_>> {
    let mut cols = line.split('\t');
    let (time, _, id, _, excerpt) = (cols.next()?, cols.next()?, cols.next()?, cols.next()?, cols.next()?);
    Some(Row { time: time.parse().ok()?, id, excerpt })
}

impl<'a> Journal<'a> {
    #[must_use]
    pub fn at(path: &'a Path) -> Self {
        Self { path }
    }

    /// Appends one hit.
    ///
    /// # Errors
    /// When the file cannot be written.
    pub fn record(&self, node: Option<i64>, place: Where, hit: &Hit) -> anyhow::Result<()> {
        use std::io::Write;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(self.path)?;
        let node = node.map_or(String::from("-"), |n| n.to_string());
        let excerpt = hit.excerpt.replace(['\t', '\n'], " ");
        writeln!(file, "{}\t{node}\t{}\t{:?}\t{excerpt}", unix_now(), hit.id, place)?;
        Ok(())
    }

    /// The ids of every hit, oldest first; empty when there is no journal yet.
    #[must_use]
    pub fn ids(&self) -> Vec<String> {
        std::fs::read_to_string(self.path)
            .map(|text| text.lines().filter_map(|l| l.split('\t').nth(2).map(str::to_owned)).collect())
            .unwrap_or_default()
    }

    /// How often each gotcha fired, the most often first.
    #[must_use]
    pub fn counts(&self) -> Vec<(String, usize)> {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for id in self.ids() {
            *counts.entry(id).or_default() += 1;
        }
        by_count(counts)
    }

    /// The commands the harness refused that no gotcha names, oldest first: the excerpts of the
    /// [`UNMATCHED`] hits. Empty when there is no journal yet.
    #[must_use]
    pub fn unmatched(&self) -> Vec<String> {
        std::fs::read_to_string(self.path)
            .map(|text| text.lines().filter_map(row).filter(|r| r.id == UNMATCHED).map(|r| r.excerpt.to_owned()).collect())
            .unwrap_or_default()
    }

    /// The ids of the last `limit` hits, newest first, for the machine notes of the next run.
    #[must_use]
    pub fn recent(&self, limit: usize) -> Vec<String> {
        let mut ids = self.ids();
        ids.reverse();
        ids.truncate(limit);
        ids
    }
}

/// What the journal says about a window: the hits by gotcha and the refused commands no gotcha
/// names by command, each the most often first.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Summary {
    /// Each gotcha that fired and how often; the [`UNMATCHED`] hits are not among them.
    pub counts: Vec<(String, usize)>,
    /// Each refused command no gotcha names and how often it was refused.
    pub unmatched: Vec<(String, usize)>,
}

impl Summary {
    /// The summary as lines a planner can cite, the counts first, each with its tally.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let counts = self.counts.iter().map(|(id, n)| format!("gotcha {id} fired {} in the last day", times(*n)));
        let unmatched = self
            .unmatched
            .iter()
            .map(|(command, n)| format!("refused command `{command}` matched no gotcha, {} in the last day", times(*n)));
        counts.chain(unmatched).collect()
    }
}

fn times(n: usize) -> String {
    if n == 1 { "once".to_owned() } else { format!("{n} times") }
}

/// The [`Summary`] of the journal at `path` over the last day; empty when there is no journal.
#[must_use]
pub fn summary(path: &Path) -> Summary {
    summary_since(path, unix_now().saturating_sub(LAST_DAY))
}

/// The [`Summary`] of the hits at or after `since`, a unix time.
#[must_use]
pub fn summary_since(path: &Path, since: u64) -> Summary {
    let Ok(text) = std::fs::read_to_string(path) else { return Summary::default() };
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut unmatched: BTreeMap<String, usize> = BTreeMap::new();
    for row in text.lines().filter_map(row).filter(|r| r.time >= since) {
        let (bucket, key) = if row.id == UNMATCHED { (&mut unmatched, row.excerpt) } else { (&mut counts, row.id) };
        *bucket.entry(key.to_owned()).or_default() += 1;
    }
    Summary { counts: by_count(counts), unmatched: by_count(unmatched) }
}

/// The map as a list, the largest count first, ties by name.
fn by_count(counts: BTreeMap<String, usize>) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = counts.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

pub(crate) fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}
