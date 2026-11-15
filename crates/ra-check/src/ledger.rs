//! The ledger of every check a project ran: one row a command, keyed by the hash of the tree it
//! ran on, so the same store measures how often and how long the suite runs and answers whether a
//! command already passed on this very tree.

#[cfg(test)]
mod tests;

use std::fs::{self, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

/// Where a project keeps its ledger: `var/checks.tsv` under its root.
#[must_use]
pub fn path(root: &Path) -> PathBuf {
    root.join("var").join("checks.tsv")
}

/// How a check ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Passed,
    Failed,
}

impl Outcome {
    fn word(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
        }
    }

    fn of_word(word: &str) -> Option<Self> {
        match word {
            "passed" => Some(Self::Passed),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// One command of one check.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// Unix seconds when the command ended.
    pub at: u64,
    /// The card the check ran for, empty when none.
    pub card: String,
    /// The stage that ran it: `worker`, `review`, `merge`, …
    pub stage: String,
    /// The hash of the working tree the command ran on, from [`tree_of`].
    pub tree: String,
    /// The commit the card's branch started from.
    pub base: String,
    pub command: String,
    /// What the toolchain was, so a pass under another compiler proves nothing.
    pub stamp: String,
    pub outcome: Outcome,
    pub seconds: f64,
    /// Tests the command ran.
    pub tests: usize,
    /// What failed it, in a line: the first failed test or compile error; empty on a pass.
    pub blame: String,
}

const FIELDS: usize = 11;

impl Row {
    fn line(&self) -> String {
        let fields = [
            self.at.to_string(),
            clean(&self.card),
            clean(&self.stage),
            clean(&self.tree),
            clean(&self.base),
            clean(&self.command),
            clean(&self.stamp),
            self.outcome.word().to_owned(),
            format!("{:.1}", self.seconds),
            self.tests.to_string(),
            clean(&self.blame),
        ];
        fields.join("\t")
    }

    fn of_line(line: &str) -> Option<Self> {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != FIELDS {
            return None;
        }
        Some(Self {
            at: fields[0].parse().ok()?,
            card: fields[1].to_owned(),
            stage: fields[2].to_owned(),
            tree: fields[3].to_owned(),
            base: fields[4].to_owned(),
            command: fields[5].to_owned(),
            stamp: fields[6].to_owned(),
            outcome: Outcome::of_word(fields[7])?,
            seconds: fields[8].parse().ok()?,
            tests: fields[9].parse().ok()?,
            blame: fields[10].to_owned(),
        })
    }
}

/// A field kept on its one line and in its one column.
fn clean(field: &str) -> String {
    field.replace(['\t', '\r', '\n'], " ")
}

/// Appends `row` to the ledger at `ledger`, making its directory when it is the first.
///
/// # Errors
/// When the ledger cannot be opened or written.
pub fn append(ledger: &Path, row: &Row) -> io::Result<()> {
    if let Some(dir) = ledger.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(ledger)?;
    writeln!(file, "{}", row.line())
}

/// Every row of the ledger, oldest first; a line that does not read as a row is skipped.
#[must_use]
pub fn rows(ledger: &Path) -> Vec<Row> {
    fs::read_to_string(ledger).unwrap_or_default().lines().filter_map(Row::of_line).collect()
}

/// The latest pass of `command` on `tree` under the toolchain `stamp`, when there is one.
#[must_use]
pub fn proven(ledger: &Path, tree: &str, command: &str, stamp: &str) -> Option<Row> {
    rows(ledger)
        .into_iter()
        .rev()
        .find(|row| row.outcome == Outcome::Passed && row.tree == tree && row.command == command && row.stamp == stamp)
}

/// The hash of the working tree at `root` as `git write-tree` would give it with every change
/// staged, untracked files included and ignored ones not; `None` outside a repository.
///
/// The staging goes through a throwaway copy of the index, so the real index is left as it was.
#[must_use]
pub fn tree_of(root: &Path) -> Option<String> {
    static COUNT: AtomicU32 = AtomicU32::new(0);
    let real = root.join(git(root, &["rev-parse", "--git-path", "index"], None)?);
    let name = format!("rhipper-check-index-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed));
    let scratch = root.join(git(root, &["rev-parse", "--git-path", &name], None)?);
    // starting from the real index spares hashing every unchanged file again
    if real.is_file() && fs::copy(&real, &scratch).is_err() {
        return None;
    }
    let tree = git(root, &["add", "-A"], Some(&scratch)).and_then(|_| git(root, &["write-tree"], Some(&scratch)));
    let _ = fs::remove_file(&scratch);
    tree
}

/// Runs git in `root`, on the index `index` when one is named; its trimmed output on success.
fn git(root: &Path, args: &[&str], index: Option<&Path>) -> Option<String> {
    let mut command = Command::new("git");
    command.current_dir(root).args(args);
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    let out = command.output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}
