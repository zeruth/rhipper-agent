//! How the catalogue grows from the journal. A command the harness refuses that no gotcha names
//! is journalled as [`UNMATCHED`](crate::UNMATCHED); when the same command comes back a third
//! time, from more than one project, it is the machine's problem and not one run's slip, so the
//! catalogue gains a drafted entry for it: a hint on the command, which the pre-tool hook turns
//! into a denial that names the fix, before the next run burns turns rediscovering it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use anyhow::Context;

use crate::{Catalogue, Conditions, ENTRY_HEAD, Verdict};

/// How many journal lines a refused command needs before it is drafted.
pub const THIRD_TIME: usize = 3;

/// How many projects those lines must come from: one project's habit is not the machine's.
pub const PROJECTS: usize = 2;

/// The comment line that heads the drafted entries in a project's file; what they came from
/// follows it, so the file says why it has them.
pub const DRAFT_MARK: &str = "# drafted from the journal: ";

/// A drafted entry: the command's first word, how often and where it was refused, and one of
/// the commands as it was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub id: String,
    pub word: String,
    pub hits: usize,
    pub projects: Vec<String>,
    pub example: String,
}

impl Draft {
    /// The message the denied worker reads: the evidence and the fix.
    #[must_use]
    pub fn message(&self) -> String {
        format!(
            "`{}` was refused by the harness {} times across {} projects ({}); it does not run here. The shells run cargo, the read-only git commands and the read-only listing commands; read files with Read, change them with Edit, search with Grep and Glob",
            self.word,
            self.hits,
            self.projects.len(),
            self.projects.join(", ")
        )
    }

    /// The entry as TOML, as [`Catalogue::promote`] appends it: a hint on the command, matched
    /// at the head of the command or after a chain operator, so the hook denies it with the fix.
    #[must_use]
    pub fn block(&self) -> String {
        let word = regex::escape(&self.word).replace('\'', "");
        format!(
            "{ENTRY_HEAD}\nid = \"{}\"\nwhere = \"command\"\npattern = '(^|&&|\\|\\||;|\\|)\\s*{word}(\\s|$)'\nmessage = \"{}\"\nremedy = {{ kind = \"hint\" }}\n",
            self.id,
            toml_basic(&self.message())
        )
    }
}

/// The drafts the refused commands warrant. Each pair is one journal line, the project it came
/// from and the command; the lines are matched on the command's first word, so the arguments
/// may differ. A word with [`THIRD_TIME`] lines from [`PROJECTS`] projects becomes one draft,
/// the most hits first.
#[must_use]
pub fn draft(refused: &[(String, String)]) -> Vec<Draft> {
    let mut by_word: BTreeMap<&str, (usize, BTreeSet<&str>, &str)> = BTreeMap::new();
    for (project, command) in refused {
        let Some(word) = command.split_whitespace().next() else { continue };
        let entry = by_word.entry(word).or_insert((0, BTreeSet::new(), command));
        entry.0 += 1;
        entry.1.insert(project);
    }
    let mut drafts: Vec<Draft> = by_word
        .into_iter()
        .filter(|(_, (hits, projects, _))| *hits >= THIRD_TIME && projects.len() >= PROJECTS)
        .map(|(word, (hits, projects, example))| Draft {
            id: id_of(word),
            word: word.to_owned(),
            hits,
            projects: projects.into_iter().map(str::to_owned).collect(),
            example: example.to_owned(),
        })
        .collect();
    drafts.sort_by(|a, b| b.hits.cmp(&a.hits).then_with(|| a.id.cmp(&b.id)));
    drafts
}

/// The id of the drafted entry for a command word: `refused-` and the word, lowered, with every
/// run of other characters as one dash.
fn id_of(word: &str) -> String {
    let mut id = String::from("refused-");
    let mut dash = false;
    for c in word.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            id.push(c);
            dash = false;
        } else if !dash {
            id.push('-');
            dash = true;
        }
    }
    id.trim_end_matches('-').to_owned()
}

/// The text as the inside of a TOML basic string.
fn toml_basic(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

impl Catalogue {
    /// Loads the catalogue at `path` as [`Self::load`] does, then brings it the entries the
    /// refused commands warrant: every [`draft`] whose id the file lacks and whose command the
    /// catalogue still lets pass is appended under a comment line naming where it came from.
    /// Nothing is written when there is nothing to draft. Returns the catalogue as it stands and
    /// the ids it gained.
    ///
    /// # Errors
    /// When the file cannot be read or written, or does not parse.
    pub fn promote(path: &Path, refused: &[(String, String)]) -> anyhow::Result<(Self, Vec<String>)> {
        let have = Self::load(path)?;
        let new: Vec<Draft> = draft(refused)
            .into_iter()
            .filter(|d| have.get(&d.id).is_none() && have.command(&d.example, Conditions::default()) == Verdict::Pass)
            .collect();
        if new.is_empty() {
            return Ok((have, Vec::new()));
        }
        let mut text = std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
        if !text.ends_with('\n') {
            text.push('\n');
        }
        let projects: BTreeSet<&str> = new.iter().flat_map(|d| d.projects.iter().map(String::as_str)).collect();
        let _ = writeln!(text, "\n{DRAFT_MARK}{} refused in {}", new.len(), projects.into_iter().collect::<Vec<_>>().join(", "));
        for d in &new {
            text.push('\n');
            text.push_str(&d.block());
        }
        std::fs::write(path, &text).with_context(|| format!("cannot write {}", path.display()))?;
        let merged = Self::parse(&text).with_context(|| path.display().to_string())?;
        Ok((merged, new.into_iter().map(|d| d.id).collect()))
    }
}
