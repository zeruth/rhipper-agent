//! The backticked names of a card, searched in the tracked sources before a run starts: the files
//! that already hold them are where the worker reads first.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

/// How many files the handoff lists.
pub(crate) const FILES_LISTED: usize = 5;

/// The shortest name worth a search: a shorter one matches everywhere.
const NAME_MIN: usize = 4;

/// The identifiers between backticks in `text`: one word of letters, digits, `_` and `::`, so a
/// command, a path or a sentence in backticks is left out. Each once, in order of appearance.
#[must_use]
pub(crate) fn backticked_names(text: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for (index, piece) in text.split('`').enumerate() {
        let is_code = index % 2 == 1;
        let name = piece.trim().trim_end_matches("()");
        let identifier = name.len() >= NAME_MIN
            && name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == ':')
            && name.chars().any(char::is_alphabetic);
        if is_code && identifier && !names.iter().any(|seen| seen == name) {
            names.push(name.to_owned());
        }
    }
    names
}

/// Up to [`FILES_LISTED`] tracked files holding the card's backticked names, the ones holding the
/// most names first. Empty when `root` is no repository or no name is found.
#[must_use]
pub(crate) fn files_holding(root: &Path, text: &str) -> Vec<String> {
    let mut hits: BTreeMap<String, usize> = BTreeMap::new();
    for name in backticked_names(text) {
        let Ok(output) = Command::new("git").args(["grep", "-l", "-I", "-w", "-F", "-e", &name]).current_dir(root).output() else {
            return Vec::new();
        };
        for file in String::from_utf8_lossy(&output.stdout).lines() {
            *hits.entry(file.to_owned()).or_default() += 1;
        }
    }
    let mut ranked: Vec<(String, usize)> = hits.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked.into_iter().take(FILES_LISTED).map(|(file, _)| file).collect()
}

/// The handoff's line for those files, empty when there are none.
#[must_use]
pub(crate) fn existing_names_note(root: &Path, text: &str) -> String {
    let files = files_holding(root, text);
    if files.is_empty() {
        return String::new();
    }
    format!("Files that already hold the card's names, read these first: {}\n", files.join(", "))
}
