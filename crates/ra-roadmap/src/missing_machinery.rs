//! The check of a card that names machinery the tree does not have. A planner writes an
//! objective around `identifiers` it assumes exist; when none of them is in the tree the card
//! builds on nothing, and the planner adds a `starts-from: nothing` line or fixes the names.

use std::fs;
use std::path::Path;

use crate::check::is_command;

/// The line that says a card builds new machinery on purpose.
pub const STARTS_FROM_NOTHING: &str = "starts-from: nothing";

/// Directories that hold no source of the project.
const SKIPPED_DIRS: [&str; 5] = [".git", "target", "var", "node_modules", ".gradle"];

/// The extensions of files an identifier is looked up in.
const SOURCE_EXTENSIONS: [&str; 12] = ["rs", "py", "js", "ts", "tsx", "java", "kt", "go", "c", "h", "cpp", "cs"];

/// The backticked identifiers of an objective, each once: a span that is one word of letters,
/// digits and `_`, or a `::` path (its last segment), with a trailing `()` dropped. Commands,
/// file paths and prose are not identifiers.
#[must_use]
pub fn identifiers(objective: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for (index, span) in objective.split('`').enumerate() {
        if index % 2 == 0 || is_command(span) {
            continue;
        }
        let span = span.trim().trim_end_matches("()");
        let Some(name) = span.rsplit("::").next() else { continue };
        let is_name = !name.is_empty()
            && name.chars().all(|c| c.is_alphanumeric() || c == '_')
            && span.split("::").all(|part| !part.is_empty() && part.chars().all(|c| c.is_alphanumeric() || c == '_'));
        if is_name && !found.iter().any(|f| f == name) {
            found.push(name.to_owned());
        }
    }
    found
}

/// The identifiers of `objective` that no source file under `root` mentions, when none of them
/// is found and `text` (the card's objective and notes) has no `starts-from: nothing` line.
/// Empty when the card names no identifier, names one that exists, or says it starts from nothing.
#[must_use]
pub fn missing(objective: &str, text: &str, root: &Path) -> Vec<String> {
    if text.lines().any(|line| line.trim().eq_ignore_ascii_case(STARTS_FROM_NOTHING)) {
        return Vec::new();
    }
    let names = identifiers(objective);
    if names.is_empty() {
        return Vec::new();
    }
    let mut remaining = names.clone();
    search(root, &mut remaining);
    if remaining.len() < names.len() { Vec::new() } else { names }
}

/// Remove from `remaining` every name some source file under `dir` contains.
fn search(dir: &Path, remaining: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if remaining.is_empty() {
            return;
        }
        let path = entry.path();
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            let name = entry.file_name();
            if !SKIPPED_DIRS.iter().any(|skipped| name == *skipped) {
                search(&path, remaining);
            }
        } else if path.extension().and_then(|e| e.to_str()).is_some_and(|e| SOURCE_EXTENSIONS.contains(&e))
            && let Ok(text) = fs::read_to_string(&path)
        {
            remaining.retain(|name| !text.contains(name.as_str()));
        }
    }
}

/// The report for the planner, or `None` when the card is fine.
#[must_use]
pub fn report(objective: &str, text: &str, root: &Path) -> Option<String> {
    let names = missing(objective, text, root);
    if names.is_empty() {
        return None;
    }
    let list = names.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ");
    Some(format!("the objective names {list}, none of which is in the tree: add a `{STARTS_FROM_NOTHING}` line or fix the names"))
}
