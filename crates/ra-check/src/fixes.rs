//! The lint fixes the compiler marks machine-applicable, read from cargo's JSON messages
//! (`--message-format=json`) and written into the files, so a warning on a card's own lines
//! is fixed without a model pass.

use std::path::Path;

use serde_json::Value;

#[cfg(test)]
mod tests;

/// One machine-applicable replacement: the bytes `start..end` of `file` become `replacement`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fix {
    /// The file as the compiler names it, relative to the workspace root, with `/`.
    pub file: String,
    /// The first and last line the span covers, 1-based.
    pub lines: (u32, u32),
    /// The span's first byte in the file.
    pub start: usize,
    /// The byte after the span's last.
    pub end: usize,
    /// The text the span becomes.
    pub replacement: String,
}

/// The machine-applicable suggestions of `json` whose spans lie inside `changed`, the
/// `(path, first, last)` ranges of a diff. A suggestion of several parts is kept whole or not at
/// all, since half of one does not compile; one reported twice (the lib and its test build) is
/// kept once. Lines that are not a compiler message are skipped.
#[must_use]
pub fn fixes(json: &str, changed: &[(String, u32, u32)]) -> Vec<Fix> {
    let mut found = Vec::new();
    for line in json.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        let message = if value["reason"] == "compiler-message" { &value["message"] } else { &value };
        collect(message, changed, &mut found);
    }
    found
}

/// The suggestions of one diagnostic and its children, each child's parts together.
fn collect(message: &Value, changed: &[(String, u32, u32)], found: &mut Vec<Fix>) {
    let parts: Vec<Fix> = message["spans"].as_array().into_iter().flatten().filter_map(fix_of).collect();
    if !parts.is_empty() && parts.iter().all(|fix| inside(fix, changed)) {
        for fix in parts {
            if !found.contains(&fix) {
                found.push(fix);
            }
        }
    }
    for child in message["children"].as_array().into_iter().flatten() {
        collect(child, changed, found);
    }
}

/// A span's replacement, when it has one the compiler calls machine-applicable.
fn fix_of(span: &Value) -> Option<Fix> {
    if span["suggestion_applicability"] != "MachineApplicable" {
        return None;
    }
    let number = |key: &str| span[key].as_u64();
    Some(Fix {
        file: span["file_name"].as_str()?.replace('\\', "/"),
        lines: (u32::try_from(number("line_start")?).ok()?, u32::try_from(number("line_end")?).ok()?),
        start: usize::try_from(number("byte_start")?).ok()?,
        end: usize::try_from(number("byte_end")?).ok()?,
        replacement: span["suggested_replacement"].as_str()?.to_owned(),
    })
}

/// Whether the span's lines all lie in one changed range of its file.
fn inside(fix: &Fix, changed: &[(String, u32, u32)]) -> bool {
    changed.iter().any(|(path, first, last)| {
        let path = path.replace('\\', "/");
        let same = path == fix.file || path.ends_with(&format!("/{}", fix.file)) || fix.file.ends_with(&format!("/{path}"));
        same && *first <= fix.lines.0 && fix.lines.1 <= *last
    })
}

/// Writes `fixes` into the files under `root`, last span first so the earlier offsets hold; a
/// span overlapping one already written, or off the file's text, is skipped. Returns how many
/// were written.
///
/// # Errors
/// A file that cannot be read or written.
pub fn apply(root: &Path, fixes: &[Fix]) -> std::io::Result<usize> {
    let mut file_list: Vec<&str> = Vec::new();
    for fix in fixes {
        if !file_list.contains(&fix.file.as_str()) {
            file_list.push(&fix.file);
        }
    }
    let mut applied = 0;
    for file in file_list {
        let path = root.join(file);
        let mut text = std::fs::read_to_string(&path)?;
        let mut spans: Vec<&Fix> = fixes.iter().filter(|fix| fix.file == file).collect();
        spans.sort_by_key(|a| std::cmp::Reverse((a.start, a.end)));
        spans.dedup();
        let mut limit = text.len();
        let mut written = 0;
        for fix in spans {
            let fits = fix.start <= fix.end && fix.end <= limit;
            if fits && text.is_char_boundary(fix.start) && text.is_char_boundary(fix.end) {
                text.replace_range(fix.start..fix.end, &fix.replacement);
                limit = fix.start;
                written += 1;
            }
        }
        if written > 0 {
            std::fs::write(&path, text)?;
            applied += written;
        }
    }
    Ok(applied)
}
