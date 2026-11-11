//! The evidence block the milestone planner reads beside the project context: what the runs
//! themselves recorded, so a plan starts from it rather than searching for it. Four sources,
//! each the newest few lines, each line clipped to [`CLIP`]: the tail of the project's look-back
//! log, the shared notes of the other projects on the machine, the ledger's costliest cards with
//! their tokens a turn, and the other projects' latest digest lines. A source that is missing or
//! empty adds nothing, and the block is empty when all are.
//!
//! The logs' format belongs to the console that writes them: a tab-separated line of the time,
//! the card or project, a kind and the text. This file reads the text column and no more, so a
//! line it cannot parse still goes in whole; the ledger is a database, so its cards arrive read.

use std::fmt::Write;
use std::path::{Path, PathBuf};

/// The heading of the block.
pub const HEADING: &str = "# Evidence";

/// The longest an evidence line gets: about forty tokens, so four sources cost a page at most.
pub const CLIP: usize = 160;

/// How many of the newest lines each source contributes.
pub const LINES_A_SOURCE: usize = 5;

/// The look-back log under a project root.
pub const RETRO_FILE: &str = "var/retro.log";

/// The shared notes under the hub.
pub const NOTES_FILE: &str = "notes.log";

/// The digests' directory under the hub.
pub const DIGEST_DIR: &str = "digest";

/// One of the ledger's costliest cards, as the caller read it.
#[derive(Debug, Clone, PartialEq)]
pub struct LedgerCard {
    pub node: i64,
    pub cost_usd: Option<f64>,
    pub turns: u64,
    pub tokens_per_turn: u64,
}

/// Where the evidence is read from.
#[derive(Debug, Clone, PartialEq)]
pub struct Sources {
    /// The project's name, as the hub files it: its own notes and digest are left out.
    pub this: String,
    pub retro: PathBuf,
    pub notes: PathBuf,
    pub digests: PathBuf,
    pub ledger: Vec<LedgerCard>,
}

impl Sources {
    /// The sources of the project at `root` with the machine's hub at `hub`; the ledger's cards
    /// are added by the caller that holds the ledger.
    #[must_use]
    pub fn under(root: &Path, hub: &Path) -> Self {
        Self {
            this: project_name(root),
            retro: root.join(RETRO_FILE),
            notes: hub.join(NOTES_FILE),
            digests: hub.join(DIGEST_DIR),
            ledger: Vec::new(),
        }
    }
}

/// The block: the heading, then one `## source` section per source that has a line, each line
/// a bullet cited by file and line number where there is one; empty without any line.
#[must_use]
pub fn block(sources: &Sources) -> String {
    let sections = [
        ("Look-backs (var/retro.log)", retro_lines(&sources.retro)),
        ("Shared machine notes (notes.log)", notes_lines(&sources.notes, &sources.this)),
        ("Costliest cards of the ledger", ledger_lines(&sources.ledger)),
        ("The other projects", digest_lines(&sources.digests, &sources.this)),
    ];
    let mut text = String::new();
    for (title, lines) in sections.iter().filter(|(_, lines)| !lines.is_empty()) {
        if text.is_empty() {
            text.push_str(HEADING);
            text.push('\n');
        }
        let _ = writeln!(text, "\n## {title}");
        for line in lines {
            let _ = writeln!(text, "- {}", clip(line));
        }
    }
    text
}

/// A line cut at [`CLIP`] characters with an ellipsis, whole when it fits.
#[must_use]
pub fn clip(line: &str) -> String {
    let line = line.trim();
    if line.chars().count() <= CLIP {
        return line.to_owned();
    }
    let mut clipped: String = line.chars().take(CLIP - 1).collect();
    clipped.push('…');
    clipped
}

/// The newest lines of the project's look-back log, oldest first, each cited `retro.log:N`
/// with the card it came from when a card is named.
fn retro_lines(path: &Path) -> Vec<String> {
    tail(
        log_lines(path)
            .map(|(number, second, text)| {
                let who = second.parse::<i64>().map_or_else(|_| String::new(), |id| format!("card #{id}: "));
                format!("retro.log:{number} {who}{text}")
            })
            .collect(),
    )
}

/// The newest shared notes of the other projects, oldest first, each cited `notes.log:N` with
/// the project that learned it.
fn notes_lines(path: &Path, this: &str) -> Vec<String> {
    tail(
        log_lines(path)
            .filter(|(_, project, _)| project != this)
            .map(|(number, project, text)| format!("notes.log:{number} ({project}) {text}"))
            .collect(),
    )
}

/// The ledger's cards as the caller ranked them, each with its cost, turns and tokens a turn.
fn ledger_lines(cards: &[LedgerCard]) -> Vec<String> {
    cards
        .iter()
        .take(LINES_A_SOURCE)
        .map(|card| {
            let cost = card.cost_usd.map_or_else(|| "an unreported amount".to_owned(), |c| format!("${c:.2}"));
            format!(
                "usage.db #{}: card #{} cost {cost} over {} turn(s), {} tokens a turn",
                card.node, card.node, card.turns, card.tokens_per_turn
            )
        })
        .collect()
}

/// The other projects' digests by name, one line each: the live run when one is going, else
/// the last card that ended, else the ready count.
fn digest_lines(dir: &Path, this: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "txt")).collect();
    files.sort();
    files
        .iter()
        .filter_map(|file| {
            let text = std::fs::read_to_string(file).ok()?;
            let field = |key: &str| text.lines().find_map(|l| l.strip_prefix(key).and_then(|rest| rest.strip_prefix(": "))).map(str::trim);
            let name = field("name").filter(|n| !n.is_empty() && *n != this)?;
            let headline = match (field("live"), field("last"), field("ready")) {
                (Some(live), _, _) => format!("live: {live}"),
                (None, Some(last), _) => format!("last: {last}"),
                (None, None, ready) => format!("{} ready, nothing run yet", ready.unwrap_or("0")),
            };
            Some(format!("{name} · {headline}"))
        })
        .take(LINES_A_SOURCE)
        .collect()
}

/// The lines of a log as the console writes them: the line number, the second column (a card
/// id in the project's log, a project name in the shared one) and the text; a line without the
/// columns is its own text, so nothing recorded is lost.
fn log_lines(path: &Path) -> impl Iterator<Item = (usize, String, String)> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            let mut cols = line.split('\t');
            match (cols.next(), cols.next(), cols.next(), cols.next()) {
                (Some(_), Some(second), Some(_), Some(text)) => (index + 1, second.to_owned(), text.to_owned()),
                _ => (index + 1, String::new(), line.to_owned()),
            }
        })
        .collect::<Vec<_>>()
        .into_iter()
}

/// The last [`LINES_A_SOURCE`] of `all`, in their order.
fn tail(mut all: Vec<String>) -> Vec<String> {
    let skip = all.len().saturating_sub(LINES_A_SOURCE);
    all.drain(..skip);
    all
}

/// The project's name as the hub files it: the directory's name.
fn project_name(root: &Path) -> String {
    std::fs::canonicalize(root)
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .or_else(|| root.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| String::from("-"))
}
