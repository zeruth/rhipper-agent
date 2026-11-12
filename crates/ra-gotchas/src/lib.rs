//! The gotchas: recurring, machine-dependent problems a worker run hits (a command the
//! allowlist refuses, a binary a console holds, a code page that mangles text), and the local
//! fix for each. The catalogue is a TOML file; the matching is regular expressions; nothing here
//! costs a model call.
//!
//! Three places use it. The pre-tool hook asks [`Catalogue::command`] what to do with a command
//! before it runs, and [`Catalogue::read`] what to do with a whole-file Read of a long file the
//! card outlines. The post-tool hook and the provider's stream reader ask [`Catalogue::scan`]
//! what a result or an answer shows. The [`Journal`] keeps every hit, so `rc gotchas` can say
//! which ones keep coming back and the next run can read [`Catalogue::notes`] about them. The
//! refused commands no gotcha names grow the catalogue through [`Catalogue::promote`].

use std::fmt::Write as _;
use std::path::Path;

use anyhow::Context;
use regex::Regex;
use serde::Deserialize;

pub mod catalog;
pub mod journal;

pub use journal::Journal;

/// The seed catalogue, written to `var/gotchas.toml` when there is none.
pub const SEED: &str = include_str!("../gotchas.toml");

/// The id the journal gives a command the harness refused that no gotcha names: not an entry of
/// the catalogue, but a hit all the same, so the catalogue can grow from it.
pub const UNMATCHED: &str = "unmatched-denied";

/// The comment line that heads the entries a later seed brought to a project's catalogue; the
/// build's version follows it, so the file says which build added what.
pub const SEED_MARK: &str = "# from the seed of ra-gotchas ";

/// The line that opens an entry in the TOML.
pub(crate) const ENTRY_HEAD: &str = "[[gotcha]]";

/// The seed's entries as text, each with its id: the blocks [`Catalogue::refresh`] appends, so
/// a project's file gains the entry as written, comments and quoting included.
fn seed_blocks() -> Vec<(String, String)> {
    SEED.split(ENTRY_HEAD)
        .skip(1)
        .map(|rest| {
            let body = rest.trim();
            let id = body
                .lines()
                .find_map(|line| line.strip_prefix("id = \""))
                .and_then(|tail| tail.split('"').next())
                .expect("every seed entry has an id")
                .to_owned();
            (id, format!("{ENTRY_HEAD}\n{body}\n"))
        })
        .collect()
}

/// Where a gotcha shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Where {
    /// A shell command the worker is about to run.
    Command,
    /// The text a shell returned.
    Result,
    /// What an edit tool answered: a miss, a stale read, a text that was not unique.
    Edit,
    /// The worker's own words between its tool calls: an edit it says it got wrong, work it
    /// says it has to do over.
    Text,
    /// The worker's final report.
    Answer,
    /// A Read of a whole file, offset and limit both missing, of a file over [`LONG_FILE`]
    /// lines the card's outline knows; the pattern is matched against the file's path.
    Read,
}

/// A file of more lines than this is read by region once the card's outline knows it.
pub const LONG_FILE: usize = 400;

/// A Read about to run, as the pre-tool hook sees it.
#[derive(Debug, Clone, Copy)]
pub struct ReadCall<'a> {
    /// The file, as the outline names it.
    pub file: &'a str,
    /// The Read names an offset or a limit.
    pub ranged: bool,
    /// The file's line count.
    pub lines: usize,
    /// The outline's items of the file with their lines, `name 120-159` each; `None` when the
    /// card's outline does not know the file.
    pub items: Option<&'a [String]>,
}

/// What to do about a gotcha.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Remedy {
    /// The message goes to the worker.
    Hint,
    /// The command runs changed: `insert` after its first word, or on its end with `append`,
    /// unless it matches `unless`; `when` limits the rewrite to a condition of the machine.
    Rewrite {
        insert: String,
        /// How many words of the command come before the insert; one unless said otherwise.
        #[serde(default = "one")]
        after: usize,
        /// The insert goes on the end of the command, after everything: a pipe, a redirect.
        #[serde(default)]
        append: bool,
        #[serde(default, with = "serde_regex_opt")]
        unless: Option<Regex>,
        #[serde(default)]
        when: Option<String>,
    },
    /// The command runs as it is; the hit is only counted.
    Allow,
}

/// One entry of the catalogue.
#[derive(Debug, Clone, Deserialize)]
pub struct Gotcha {
    pub id: String,
    #[serde(rename = "where")]
    pub place: Where,
    #[serde(with = "serde_regex")]
    pub pattern: Regex,
    pub message: String,
    pub remedy: Remedy,
}

/// The catalogue.
#[derive(Debug, Clone, Deserialize)]
pub struct Catalogue {
    #[serde(rename = "gotcha", default)]
    pub entries: Vec<Gotcha>,
}

/// What the machine looks like right now, for the remedies that depend on it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Conditions {
    /// A console holds the binary.
    pub console_running: bool,
}

/// What the pre-tool hook does with a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The command runs as it is.
    Pass,
    /// The command is refused; the worker reads the reason.
    Deny { id: String, reason: String },
    /// The command runs changed; the worker reads why.
    Rewrite { id: String, command: String, reason: String },
}

/// One hit: the gotcha and what it matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub id: String,
    pub message: String,
    /// The line that matched, clipped.
    pub excerpt: String,
}

impl Catalogue {
    /// Parses a catalogue from its TOML text.
    ///
    /// # Errors
    /// When the text is not a catalogue, or a pattern is not a regular expression.
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        toml::from_str(text).context("the gotchas file does not parse")
    }

    /// The seed catalogue.
    pub fn seed() -> Self {
        Self::parse(SEED).expect("the seed catalogue parses")
    }

    /// Loads the catalogue at `path`, writing the seed there first when there is none.
    ///
    /// # Errors
    /// When the file cannot be read or written, or does not parse.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, SEED).with_context(|| format!("cannot write {}", path.display()))?;
        }
        let text = std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
        Self::parse(&text).with_context(|| path.display().to_string())
    }

    /// Loads the catalogue at `path` as [`Self::load`] does, then brings it the seed's new
    /// entries: every seed entry whose id the file lacks is appended under a comment line
    /// naming the build that brought it, and the entries the file has, changed or not, stay its
    /// own byte for byte. Nothing is written when nothing is missing. Returns the catalogue as
    /// it stands and the ids it gained.
    ///
    /// # Errors
    /// When the file cannot be read or written, or does not parse.
    pub fn refresh(path: &Path) -> anyhow::Result<(Self, Vec<String>)> {
        let have = Self::load(path)?;
        let missing: Vec<(String, String)> = seed_blocks().into_iter().filter(|(id, _)| have.get(id).is_none()).collect();
        if missing.is_empty() {
            return Ok((have, Vec::new()));
        }
        let mut text = std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
        if !text.ends_with('\n') {
            text.push('\n');
        }
        let _ = writeln!(text, "\n{SEED_MARK}{}, {} new", env!("CARGO_PKG_VERSION"), missing.len());
        for (_, block) in &missing {
            text.push('\n');
            text.push_str(block);
        }
        std::fs::write(path, &text).with_context(|| format!("cannot write {}", path.display()))?;
        let merged = Self::parse(&text).with_context(|| path.display().to_string())?;
        Ok((merged, missing.into_iter().map(|(id, _)| id).collect()))
    }

    /// How many entries of a catalogue's text came from the seed after the file was written:
    /// the ones under its [`SEED_MARK`] lines.
    #[must_use]
    pub fn from_seed_later(text: &str) -> usize {
        text.find(SEED_MARK).map_or(0, |at| text[at..].matches(ENTRY_HEAD).count())
    }

    /// The entry with `id`.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Gotcha> {
        self.entries.iter().find(|g| g.id == id)
    }

    /// What to do with a command about to run: the first hint or allow that matches decides;
    /// the rewrites before it compose, each on the command as the one before left it.
    #[must_use]
    pub fn command(&self, command: &str, conditions: Conditions) -> Verdict {
        let mut current = command.to_owned();
        let mut rewritten: Option<(String, Vec<String>)> = None;
        for gotcha in self.entries.iter().filter(|g| g.place == Where::Command) {
            if !gotcha.pattern.is_match(&current) {
                continue;
            }
            match &gotcha.remedy {
                Remedy::Hint => return Verdict::Deny { id: gotcha.id.clone(), reason: gotcha.message.clone() },
                Remedy::Allow => break,
                Remedy::Rewrite { insert, after, append, unless, when } => {
                    let applies = match when.as_deref() {
                        Some("console-running") => conditions.console_running,
                        Some(_) => false,
                        None => true,
                    };
                    if !applies || unless.as_ref().is_some_and(|u| u.is_match(&current)) {
                        continue;
                    }
                    current = if *append { format!("{} {insert}", current.trim_end()) } else { insert_after(&current, *after, insert) };
                    match &mut rewritten {
                        None => rewritten = Some((gotcha.id.clone(), vec![gotcha.message.clone()])),
                        Some((_, reasons)) => reasons.push(gotcha.message.clone()),
                    }
                }
            }
        }
        match rewritten {
            Some((id, reasons)) => Verdict::Rewrite { id, command: current, reason: reasons.join("; ") },
            None => Verdict::Pass,
        }
    }

    /// What to do with a Read about to run: a whole-file Read of a file over [`LONG_FILE`] lines
    /// the card's outline knows is denied by the first hint whose pattern matches its path, the
    /// outline's items in the reason, so the next Read takes a region; a ranged Read, a short
    /// file and a file the outline does not know pass.
    #[must_use]
    pub fn read(&self, call: ReadCall<'_>) -> Verdict {
        let Some(items) = call.items.filter(|_| !call.ranged && call.lines > LONG_FILE) else { return Verdict::Pass };
        for gotcha in self.entries.iter().filter(|g| g.place == Where::Read && g.pattern.is_match(call.file)) {
            match gotcha.remedy {
                Remedy::Hint => {
                    let mut reason = format!("{}: {} is {} lines", gotcha.message, call.file, call.lines);
                    if !items.is_empty() {
                        let _ = write!(reason, "; its items: {}", items.join(", "));
                    }
                    return Verdict::Deny { id: gotcha.id.clone(), reason };
                }
                Remedy::Allow => break,
                Remedy::Rewrite { .. } => {}
            }
        }
        Verdict::Pass
    }

    /// The Read gotchas a refused Read's result names, each once: the harness answers the call
    /// with the hook's reason, which opens with the gotcha's message.
    #[must_use]
    pub fn refused_read(&self, result: &str) -> Vec<Hit> {
        self.entries
            .iter()
            .filter(|g| g.place == Where::Read && result.contains(&g.message))
            .map(|g| Hit { id: g.id.clone(), message: g.message.clone(), excerpt: clip(result.lines().next().unwrap_or("").trim(), 160) })
            .collect()
    }

    /// The gotchas a result or an answer shows, each once, with the first line that matched.
    #[must_use]
    pub fn scan(&self, place: Where, text: &str) -> Vec<Hit> {
        self.entries
            .iter()
            .filter(|g| g.place == place)
            .filter_map(|g| {
                let line = text.lines().find(|l| g.pattern.is_match(l)).or_else(|| g.pattern.is_match(text).then_some(""))?;
                Some(Hit { id: g.id.clone(), message: g.message.clone(), excerpt: clip(line.trim(), 160) })
            })
            .collect()
    }

    /// The machine notes for a run: the messages of the gotchas in `ids`, each once, as a short
    /// block for the system prompt. Empty when there are none.
    pub fn notes(&self, ids: &[String]) -> String {
        let mut seen = Vec::new();
        let mut lines = Vec::new();
        for id in ids {
            if seen.contains(id) {
                continue;
            }
            if let Some(g) = self.get(id) {
                seen.push(id.clone());
                lines.push(format!("- {}", g.message));
            }
        }
        if lines.is_empty() { String::new() } else { format!("About this machine, from earlier runs:\n{}", lines.join("\n")) }
    }
}

/// The command with `insert` after its first `after` words, and the rest as it was.
fn insert_after(command: &str, after: usize, insert: &str) -> String {
    let words: Vec<&str> = command.split_whitespace().collect();
    let at = after.clamp(1, words.len());
    let mut out = words[..at].join(" ");
    out.push(' ');
    out.push_str(insert);
    if at < words.len() {
        out.push(' ');
        out.push_str(&words[at..].join(" "));
    }
    out
}

fn one() -> usize {
    1
}

fn clip(text: &str, room: usize) -> String {
    if text.chars().count() <= room { text.to_owned() } else { format!("{}…", text.chars().take(room - 1).collect::<String>()) }
}

/// Serde for a regular expression in a TOML string.
mod serde_regex {
    use regex::Regex;
    use serde::{Deserialize, Deserializer};

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Regex, D::Error> {
        let text = String::deserialize(d)?;
        Regex::new(&text).map_err(serde::de::Error::custom)
    }
}

/// Serde for an optional regular expression.
mod serde_regex_opt {
    use regex::Regex;
    use serde::{Deserialize, Deserializer};

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Regex>, D::Error> {
        let text: Option<String> = Option::deserialize(d)?;
        text.map(|t| Regex::new(&t).map_err(serde::de::Error::custom)).transpose()
    }
}

#[cfg(test)]
mod tests;
