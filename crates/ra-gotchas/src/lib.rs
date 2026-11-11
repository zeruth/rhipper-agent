//! The gotchas: recurring, machine-dependent problems a worker run hits (a command the
//! allowlist refuses, a binary a console holds, a code page that mangles text), and the local
//! fix for each. The catalogue is a TOML file; the matching is regular expressions; nothing here
//! costs a model call.
//!
//! Three places use it. The pre-tool hook asks [`Catalogue::command`] what to do with a command
//! before it runs. The post-tool hook and the provider's stream reader ask [`Catalogue::scan`]
//! what a result or an answer shows. The [`Journal`] keeps every hit, so `rc gotchas` can say
//! which ones keep coming back and the next run can read [`Catalogue::notes`] about them.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Context;
use regex::Regex;
use serde::Deserialize;

/// The seed catalogue, written to `var/gotchas.toml` when there is none.
pub const SEED: &str = include_str!("../gotchas.toml");

/// Where a gotcha shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Where {
    /// A shell command the worker is about to run.
    Command,
    /// The text a tool returned.
    Result,
    /// The worker's final report.
    Answer,
}

/// What to do about a gotcha.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Remedy {
    /// The message goes to the worker.
    Hint,
    /// The command runs changed: `insert` after its first word, unless it matches `unless`;
    /// `when` limits the rewrite to a condition of the machine.
    Rewrite {
        insert: String,
        /// How many words of the command come before the insert; one unless said otherwise.
        #[serde(default = "one")]
        after: usize,
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

    /// The entry with `id`.
    pub fn get(&self, id: &str) -> Option<&Gotcha> {
        self.entries.iter().find(|g| g.id == id)
    }

    /// What to do with a command about to run: the first command gotcha that matches decides.
    pub fn command(&self, command: &str, conditions: Conditions) -> Verdict {
        for gotcha in self.entries.iter().filter(|g| g.place == Where::Command && g.pattern.is_match(command)) {
            match &gotcha.remedy {
                Remedy::Hint => return Verdict::Deny { id: gotcha.id.clone(), reason: gotcha.message.clone() },
                Remedy::Allow => return Verdict::Pass,
                Remedy::Rewrite { insert, after, unless, when } => {
                    let applies = match when.as_deref() {
                        Some("console-running") => conditions.console_running,
                        Some(_) => false,
                        None => true,
                    };
                    if !applies || unless.as_ref().is_some_and(|u| u.is_match(command)) {
                        continue;
                    }
                    return Verdict::Rewrite {
                        id: gotcha.id.clone(),
                        command: insert_after(command, *after, insert),
                        reason: gotcha.message.clone(),
                    };
                }
            }
        }
        Verdict::Pass
    }

    /// The gotchas a result or an answer shows, each once, with the first line that matched.
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

/// The journal of hits: one line per hit in a file, `time<TAB>node<TAB>id<TAB>where<TAB>excerpt`.
pub struct Journal<'a> {
    path: &'a Path,
}

impl<'a> Journal<'a> {
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
    pub fn ids(&self) -> Vec<String> {
        std::fs::read_to_string(self.path)
            .map(|text| text.lines().filter_map(|l| l.split('\t').nth(2).map(str::to_owned)).collect())
            .unwrap_or_default()
    }

    /// How often each gotcha fired, the most often first.
    pub fn counts(&self) -> Vec<(String, usize)> {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for id in self.ids() {
            *counts.entry(id).or_default() += 1;
        }
        let mut out: Vec<(String, usize)> = counts.into_iter().collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out
    }

    /// The ids of the last `limit` hits, newest first, for the machine notes of the next run.
    pub fn recent(&self, limit: usize) -> Vec<String> {
        let mut ids = self.ids();
        ids.reverse();
        ids.truncate(limit);
        ids
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
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
