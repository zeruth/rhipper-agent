//! The files edited after a background build started: its result was read from the tree as it
//! stood at the start, so the edits made while it ran are not covered by it.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Directories that hold state or build output, not the card's files.
const SKIPPED: [&str; 4] = [".git", "target", "var", "node_modules"];

fn walk(root: &Path, dir: &Path, started: SystemTime, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path: PathBuf = entry.path();
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            if !entry.file_name().to_str().is_some_and(|n| SKIPPED.contains(&n)) {
                walk(root, &path, started, out);
            }
        } else if kind.is_file()
            && entry.metadata().and_then(|m| m.modified()).is_ok_and(|m| m > started)
            && let Ok(rel) = path.strip_prefix(root)
        {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// The files under `root` modified after `started`, sorted, relative to `root` with `/`.
#[must_use]
pub(crate) fn edited_since(root: &Path, started: SystemTime) -> Vec<String> {
    let mut out = Vec::new();
    walk(root, root, started, &mut out);
    out.sort();
    out
}

/// The line a build's result carries when `files` were edited after it started, `None` when none were.
#[must_use]
pub(crate) fn note(files: &[String]) -> Option<String> {
    if files.is_empty() {
        return None;
    }
    Some(format!("edited after this build started, not covered by its result: {}", files.join(", ")))
}
