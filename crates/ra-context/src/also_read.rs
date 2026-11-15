//! The files a card's named symbols lead to: where the identifiers its text puts in backticks
//! are defined or called, outside the files the card names, so the worker finds them without
//! a search of its own.

use std::path::Path;

/// How many files the "also read" line names at most.
pub const ALSO_READ_CAP: usize = 8;

/// The directories a search never enters: build output, worktrees and state, vendored code.
const SKIPPED_DIRS: &[&str] = &["target", "var", "node_modules", "build", "out", "dist", "vendor"];

/// The extensions of the source files a search reads.
const SOURCE_EXTENSIONS: &[&str] = &["rs", "py", "kt", "kts", "java", "ts", "tsx", "js", "go", "c", "h", "cpp", "hpp", "cs", "swift"];

/// A file past this size is generated or data, not code a worker reads.
const MAX_FILE_BYTES: u64 = 512 * 1024;

/// How many files a search reads at most, so a huge tree costs a bounded walk.
const MAX_FILES_READ: usize = 5000;

/// The words that open a definition in the languages the search reads.
const DEFINING_WORDS: &[&str] = &[
    "fn",
    "struct",
    "enum",
    "trait",
    "type",
    "const",
    "static",
    "mod",
    "impl",
    "union",
    "macro_rules",
    "class",
    "def",
    "interface",
    "object",
    "fun",
    "func",
];

/// The symbols a card's text names in backticks: identifiers that read as code (`snake_case`,
/// `CamelCase`, a path's last segment), not plain words, commands or file names.
#[must_use]
pub fn symbols_of(text: &str) -> Vec<String> {
    let mut symbols: Vec<String> = Vec::new();
    for span in text.split('`').skip(1).step_by(2) {
        let span = span.trim().trim_end_matches("()").trim_end_matches('!');
        if span.contains(char::is_whitespace) || span.contains(['/', '\\', '.']) {
            continue;
        }
        let name = span.rsplit("::").next().unwrap_or(span);
        let is_identifier =
            name.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') && name.chars().all(|c| c.is_alphanumeric() || c == '_');
        let reads_as_code = name.contains('_') || name.chars().any(char::is_uppercase);
        if is_identifier && reads_as_code && name.len() >= 3 && !symbols.iter().any(|s| s == name) {
            symbols.push(name.to_owned());
        }
    }
    symbols
}

/// The files under `workspace` that define or use a symbol `text` names, outside `files`:
/// the defining files first, then the callers, each group by path, at most [`ALSO_READ_CAP`].
/// Paths are relative with `/`.
#[must_use]
pub fn also_read(workspace: &Path, text: &str, files: &[String]) -> Vec<String> {
    let symbols = symbols_of(text);
    if symbols.is_empty() {
        return Vec::new();
    }
    let named: Vec<String> =
        files.iter().map(|f| f.trim().trim_start_matches('+').replace('\\', "/").trim_end_matches('/').to_owned()).collect();
    let on_card = |path: &str| named.iter().any(|f| path == f || path.strip_prefix(f.as_str()).is_some_and(|rest| rest.starts_with('/')));
    let mut sources = Vec::new();
    collect_sources(workspace, workspace, &mut sources);
    sources.sort();
    // (0 for a definition, 1 for a use, the path)
    let mut found: Vec<(u8, String)> = Vec::new();
    for path in sources {
        if on_card(&path) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(workspace.join(&path)) else { continue };
        let mut rank = None;
        for symbol in &symbols {
            match standing(&content, symbol) {
                Some(0) => {
                    rank = Some(0);
                    break;
                }
                Some(r) => rank = Some(rank.map_or(r, |old: u8| old.min(r))),
                None => {}
            }
        }
        if let Some(rank) = rank {
            found.push((rank, path));
        }
    }
    found.sort();
    found.into_iter().take(ALSO_READ_CAP).map(|(_, path)| path).collect()
}

/// How `content` stands to `symbol`: 0 when it defines it, 1 when it only uses it.
fn standing(content: &str, symbol: &str) -> Option<u8> {
    if !has_word(content, symbol) {
        return None;
    }
    let defines = content.lines().any(|line| {
        let words: Vec<&str> = line.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| !w.is_empty()).collect();
        words.windows(2).any(|pair| pair[1] == symbol && DEFINING_WORDS.contains(&pair[0]))
    });
    Some(u8::from(!defines))
}

/// Whether `word` stands in `text` as a whole identifier, not inside a longer one.
fn has_word(text: &str, word: &str) -> bool {
    let is_part = |c: char| c.is_alphanumeric() || c == '_';
    text.match_indices(word).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + word.len()..].chars().next();
        !before.is_some_and(is_part) && !after.is_some_and(is_part)
    })
}

/// The source files under `dir`, relative to `root`, past the hidden and skipped directories.
fn collect_sources(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        if out.len() >= MAX_FILES_READ {
            return;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(kind) = entry.file_type() else { continue };
        let path = entry.path();
        if kind.is_dir() {
            if !name.starts_with('.') && !SKIPPED_DIRS.contains(&name.as_str()) {
                collect_sources(root, &path, out);
            }
        } else if kind.is_file()
            && path.extension().and_then(|e| e.to_str()).is_some_and(|e| SOURCE_EXTENSIONS.contains(&e))
            && entry.metadata().is_ok_and(|m| m.len() <= MAX_FILE_BYTES)
            && let Ok(relative) = path.strip_prefix(root)
        {
            out.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
}
