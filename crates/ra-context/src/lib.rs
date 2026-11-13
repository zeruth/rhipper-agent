//! The context of a run: what every task needs to know about the project, and what one task
//! has learned across its runs. Nothing here costs a model call; the point is that a worker
//! spends its tokens on the change, not on finding out where the change lives.
//!
//! Two kinds of file. [`PROJECT_FILE`] beside the roadmap is the project context: the layout,
//! the build and the checks, the conventions. It is drafted once from the workspace, edited by
//! hand after, and sent with every run. `var/context/<id>.md` is a task's context: the brief of
//! the card, an [`outline`] of each file the card names (the items and their lines, so the
//! worker reads regions rather than whole files), and a [`Handoff`] from each run, so the next
//! run of the card starts where the last one stopped. A task's file lives until the card is
//! done; [`TaskContext::prune`] removes the rest.

use std::fmt::Write;
use std::path::{Path, PathBuf};

use anyhow::Context;

pub mod digest;
pub mod plan;

/// The project context, beside the roadmap file.
pub const PROJECT_FILE: &str = "rhipper.context.md";

/// Where the tasks' files live, under the workspace.
pub const CONTEXT_DIR: &str = "var/context";

/// The heading under which the active gambits ride with the project context: the person's
/// standing direction, which a run, the planner and a reassessment read as such.
pub const DIRECTION_HEADING: &str = "## The person's standing direction";

/// The project context with the person's standing direction after it, one line per active
/// gambit as the roadmap lists them; the context alone when there is none, and the direction
/// alone when the project file is still empty. The file is not touched: the gambits live in the
/// roadmap and change there.
#[must_use]
pub fn with_direction(project: &str, direction: &[String]) -> String {
    if direction.is_empty() {
        return project.to_owned();
    }
    let mut text = project.trim_end().to_owned();
    if !text.is_empty() {
        text.push_str("\n\n");
    }
    text.push_str(DIRECTION_HEADING);
    text.push('\n');
    for line in direction {
        let _ = writeln!(text, "- {line}");
    }
    text
}

/// The headings of the sections every run keeps, whatever its card names: the build, the
/// conventions and the person's standing direction. A heading matches when it contains one of
/// these, case aside.
const KEPT_SECTIONS: [&str; 3] = ["build", "convention", "standing direction"];

/// The project context cut to a card: the title and lead, the sections whose heading is in
/// [`KEPT_SECTIONS`], the sections whose text names one of the card's files or crates, and the
/// layout section with the lines of the other crates dropped (a line that names a path in
/// backticks and none of the card's; a line that names no path stays). A card with no files
/// keeps the whole context, and so does one with a file outside any crate (`gw.ps1`,
/// `src/main.rs`): nothing says which part of it the worker needs.
#[must_use]
pub fn cut_to(project: &str, files: &[String]) -> String {
    if files.is_empty() || files.iter().any(|f| crate_of(f).is_empty()) {
        return project.to_owned();
    }
    let names = names_of(files);
    let matches_name = |line: &str| names.iter().any(|n| line.contains(n.as_str()));
    let mut out = String::new();
    for section in sections_of(project) {
        let heading = section.lines().next().unwrap_or_default();
        let lower = heading.to_lowercase();
        let keep = if !heading.starts_with("## ") || KEPT_SECTIONS.iter().any(|k| lower.contains(k)) {
            Some(section.clone())
        } else if lower.contains("layout") {
            let lines: Vec<&str> = section.lines().filter(|l| !(l.starts_with("- ") && names_path(l)) || matches_name(l)).collect();
            Some(lines.join("\n"))
        } else {
            section.lines().any(matches_name).then(|| section.clone())
        };
        if let Some(text) = keep {
            out.push_str(text.trim_end());
            out.push_str("\n\n");
        }
    }
    format!("{}\n", out.trim_end())
}

/// The context split at its `## ` headings: the lead before the first, then each section with
/// its heading as its first line.
fn sections_of(project: &str) -> Vec<String> {
    let mut sections: Vec<String> = Vec::new();
    for line in project.lines() {
        if line.starts_with("## ") || sections.is_empty() {
            sections.push(String::new());
        }
        let last = sections.last_mut().expect("a section was just pushed");
        last.push_str(line);
        last.push('\n');
    }
    sections
}

/// What a layout line or a section names a card by: each file, the crate path of each file
/// without its trailing slash (`crates/ra-tools`) and the crate's bare name (`ra-tools`).
fn names_of(files: &[String]) -> Vec<String> {
    let mut names = Vec::new();
    for file in files {
        names.push(file.clone());
        let prefix = crate_of(file);
        let prefix = prefix.trim_end_matches('/');
        if !prefix.is_empty() {
            names.push(prefix.to_owned());
            if let Some(bare) = prefix.rsplit('/').next() {
                names.push(bare.to_owned());
            }
        }
    }
    names
}

/// Whether a line names a path in backticks: a layout line of a crate or a directory.
fn names_path(line: &str) -> bool {
    line.split('`').skip(1).step_by(2).any(|token| token.contains('/') && !token.contains(' '))
}

/// `notes` with the lines already said by `known` dropped, so a fact rides once in a prompt:
/// a line is said when, letters and digits alone and case aside, it is a line of `known` or an
/// earlier line of `notes`. A heading whose lines all went goes with them.
#[must_use]
pub fn dedupe_notes(notes: &str, known: &str) -> String {
    let said = |text: &str| text.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_lowercase();
    // line by line, not the context as one string: a short bullet is not said by a substring
    let mut seen: Vec<String> = known.lines().map(said).filter(|key| !key.is_empty()).collect();
    let mut blocks: Vec<String> = Vec::new();
    // a block is a heading and its bullets; one whose bullets all went goes with them
    for block in notes.split("\n\n") {
        let mut kept: Vec<&str> = Vec::new();
        let mut bullets = 0;
        for line in block.lines() {
            if let Some(text) = line.strip_prefix("- ") {
                let key = said(text);
                if key.is_empty() || seen.contains(&key) {
                    continue;
                }
                seen.push(key);
                bullets += 1;
            }
            kept.push(line);
        }
        if bullets > 0 {
            blocks.push(kept.join("\n"));
        }
    }
    blocks.join("\n\n")
}

/// How many items an outline names before it counts the rest.
const OUTLINE_ROOM: usize = 60;

/// How many handoffs a prompt carries, newest last; the file keeps [`HANDOFFS_KEPT`].
const HANDOFFS_SHOWN: usize = 3;

/// How many handoffs a task's file keeps.
const HANDOFFS_KEPT: usize = 10;

/// How many lines of a report a handoff keeps.
const REPORT_LINES: usize = 40;

/// The project context: the text of [`PROJECT_FILE`], or a draft of it from the workspace.
pub struct Project;

impl Project {
    /// The project context, when the file exists and says something.
    #[must_use]
    pub fn load(workspace: &Path) -> Option<String> {
        std::fs::read_to_string(workspace.join(PROJECT_FILE)).ok().filter(|t| !t.trim().is_empty())
    }

    /// The project context, drafting and writing the file first when there is none.
    ///
    /// # Errors
    /// When the file cannot be written.
    pub fn ensure(workspace: &Path) -> anyhow::Result<String> {
        if let Some(text) = Self::load(workspace) {
            return Ok(text);
        }
        let draft = Self::draft(workspace);
        std::fs::write(workspace.join(PROJECT_FILE), &draft).with_context(|| format!("cannot write {PROJECT_FILE}"))?;
        Ok(draft)
    }

    /// A draft of the project context from the workspace itself: the name, the first paragraph
    /// of the README, the crates of a Cargo workspace with their descriptions, the checks and
    /// the conventions the manifest shows. Short on purpose: it rides on every run.
    #[must_use]
    pub fn draft(workspace: &Path) -> String {
        let manifest = read_toml(&workspace.join("Cargo.toml"));
        let (title, lead) = readme_lead(workspace).unwrap_or_default();
        // the package's name, else the README's title, else the directory's own name
        let name = manifest
            .as_ref()
            .and_then(|m| m.get("package")?.get("name")?.as_str().map(str::to_owned))
            .or(title)
            .or_else(|| std::fs::canonicalize(workspace).ok()?.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "the project".to_owned());
        let mut text = format!("# {name}\n\n");
        if let Some(lead) = lead {
            text.push_str(&lead);
            text.push_str("\n\n");
        }
        let members = members_of(workspace, manifest.as_ref());
        if !members.is_empty() {
            text.push_str("## Layout\n\n");
            for (path, description) in &members {
                let _ = writeln!(text, "- `{path}` — {description}");
            }
            text.push('\n');
        }
        if manifest.is_some() {
            text.push_str("## Build and checks\n\n");
            text.push_str(
                "- Every backticked command in a card's acceptance is its check (`cargo test -p x`, `python tools/x.py`, `.\\gw.ps1 :task`); several run in the order written and stop at the first failure; `cargo test --workspace -q` when none is named. Run them before reporting.\n",
            );
            if manifest.as_ref().and_then(|m| m.get("workspace")?.get("lints")?.get("clippy")).is_some() {
                text.push_str("- `cargo clippy --workspace` warns with the pedantic set; a change adds no warnings of its own.\n");
            }
            if let Some(edition) = manifest.as_ref().and_then(|m| {
                m.get("workspace").and_then(|w| w.get("package")).or_else(|| m.get("package"))?.get("edition")?.as_str().map(str::to_owned)
            }) {
                let _ = writeln!(text, "- Rust edition {edition}.");
            }
            text.push('\n');
        }
        text.push_str("## Conventions\n\n");
        text.push_str("- Match the surrounding code: its comment density, naming and idiom. Doc comments say what and why, not how.\n");
        text.push_str("- Read the files the card names first, by the regions the outline points at; search before exploring.\n");
        text.push_str("- Run the check the card names before reporting, and report what changed and what was verified, briefly.\n\n");
        let _ = writeln!(text, "_Drafted by `rc context --init` from the workspace. Edit it by hand: every run reads {PROJECT_FILE}._");
        text
    }
}

/// The root manifest as TOML, when it reads and parses.
fn read_toml(path: &Path) -> Option<toml::Value> {
    std::fs::read_to_string(path).ok()?.parse::<toml::Value>().ok()
}

/// The README's title and its first paragraph after it (up to six lines); `None` without a
/// README, and a title or a lead of `None` when the file has none.
fn readme_lead(workspace: &Path) -> Option<(Option<String>, Option<String>)> {
    let text = ["README.md", "README", "readme.md"].iter().find_map(|n| std::fs::read_to_string(workspace.join(n)).ok())?;
    let mut title = None;
    let mut lines = Vec::new();
    for line in text.lines().map(str::trim) {
        if let Some(heading) = line.strip_prefix("# ") {
            title.get_or_insert_with(|| heading.trim().to_owned());
            continue;
        }
        if line.starts_with('#') || (line.is_empty() && lines.is_empty()) {
            continue;
        }
        if line.is_empty() || lines.len() == 6 {
            break;
        }
        lines.push(line.to_owned());
    }
    Some((title, (!lines.is_empty()).then(|| lines.join(" "))))
}

/// The members of a Cargo workspace as `(path, description)`, in manifest order; a member
/// without a description gives the first `//!` line of its root source. A single package
/// stands alone.
fn members_of(workspace: &Path, manifest: Option<&toml::Value>) -> Vec<(String, String)> {
    let Some(manifest) = manifest else { return Vec::new() };
    let globs: Vec<String> = manifest
        .get("workspace")
        .and_then(|w| w.get("members"))
        .and_then(toml::Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
        .unwrap_or_default();
    let mut dirs: Vec<PathBuf> = Vec::new();
    for glob in &globs {
        match glob.strip_suffix("/*") {
            Some(parent) => {
                if let Ok(entries) = std::fs::read_dir(workspace.join(parent)) {
                    let mut found: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.join("Cargo.toml").exists()).collect();
                    found.sort();
                    dirs.extend(found);
                }
            }
            None => dirs.push(workspace.join(glob)),
        }
    }
    if dirs.is_empty() && manifest.get("package").is_some() {
        dirs.push(workspace.to_path_buf());
    }
    dirs.into_iter()
        .filter_map(|dir| {
            let package = read_toml(&dir.join("Cargo.toml"))?;
            let name = package.get("package")?.get("name")?.as_str()?.to_owned();
            let description = package
                .get("package")
                .and_then(|p| p.get("description"))
                .and_then(toml::Value::as_str)
                .map(str::to_owned)
                .or_else(|| doc_line(&dir.join("src/lib.rs")))
                .or_else(|| doc_line(&dir.join("src/main.rs")))
                .unwrap_or_else(|| name.clone());
            let shown = dir.strip_prefix(workspace).map_or(name, |p| p.to_string_lossy().replace('\\', "/"));
            Some((shown, description))
        })
        .collect()
}

/// The first `//!` line of a source file, without the marker.
fn doc_line(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines().find_map(|l| l.strip_prefix("//!").map(|d| d.trim().trim_end_matches('.').to_owned())).filter(|d| !d.is_empty())
}

/// The outline of one file: its line count, its first doc line, and the items it declares with
/// their line numbers, so a reader opens the region it needs and not the file. Rust files give
/// their functions, types, traits, impls and modules at the top level and one level in; Markdown
/// its headings; TOML its tables; anything else the definitions at the start of a line.
#[must_use]
pub fn outline(path: &Path, text: &str) -> String {
    outline_marked(path, text, &[])
}

/// The outline with the items that hold a changed line marked `*`: `changed` lists inclusive
/// 1-based line ranges, as [`changed_lines`] reads them from a diff, and an item reaches from
/// its line to the line before the next one. A worker returning to a card in revision finds its
/// own change first.
#[must_use]
pub fn outline_marked(path: &Path, text: &str, changed: &[(usize, usize)]) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let items = items(path, text);
    let hidden = items.len().saturating_sub(OUTLINE_ROOM);
    let mut out = format!("{} ({} lines)", path.display().to_string().replace('\\', "/"), lines.len());
    if extension == "rs"
        && let Some(doc) = lines.iter().find_map(|l| l.strip_prefix("//!").map(str::trim)).filter(|d| !d.is_empty())
    {
        let _ = write!(out, " — {}", clip(doc, 140));
    }
    out.push('\n');
    for (start, end, item) in items.iter().take(OUTLINE_ROOM) {
        let mark = if changed.iter().any(|&(from, to)| from <= *end && to >= *start) { '*' } else { ' ' };
        let _ = writeln!(out, "{mark} {start:>5}  {}", clip(item, 110));
    }
    if hidden > 0 {
        let _ = writeln!(out, "         … and {hidden} more");
    }
    out
}

/// The items of a file with the lines each owns, 1-based and inclusive: an item reaches from
/// its line to the line before the next one, the last to the end of the file. The same items
/// the outline lists, for a hint that names a range to read.
#[must_use]
pub fn items(path: &Path, text: &str) -> Vec<(usize, usize, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let mut items: Vec<(usize, String)> = Vec::new();
    // the tests module of a Rust file is not where a change lands; its functions stay out
    let mut in_tests = false;
    for (number, line) in lines.iter().enumerate() {
        if extension == "rs" {
            if !in_tests && line.starts_with("mod tests") && line.trim_end().ends_with('{') {
                in_tests = true;
                continue;
            }
            if in_tests {
                in_tests = line.trim_end() != "}";
                continue;
            }
        }
        let item = match extension {
            "rs" => rust_item(line),
            "md" => line.starts_with('#').then(|| line.trim().to_owned()),
            "toml" => line.starts_with('[').then(|| line.trim().to_owned()),
            _ => generic_item(line),
        };
        let Some(item) = item else { continue };
        items.push((number + 1, item));
    }
    let ends: Vec<usize> = (0..items.len()).map(|i| items.get(i + 1).map_or(lines.len().max(items[i].0), |(next, _)| next - 1)).collect();
    items.into_iter().zip(ends).map(|((start, item), end)| (start, end, item)).collect()
}

/// The items of a file as a Read hint names them, `parse 149-151` each, the name an item
/// declares or the item itself, as many as an outline shows and a count of the rest.
#[must_use]
pub fn item_ranges(path: &Path, text: &str) -> Vec<String> {
    let items = items(path, text);
    let mut ranges: Vec<String> =
        items.iter().take(OUTLINE_ROOM).map(|(start, end, item)| format!("{} {start}-{end}", item_name(item).unwrap_or(item))).collect();
    if items.len() > OUTLINE_ROOM {
        ranges.push(format!("… and {} more", items.len() - OUTLINE_ROOM));
    }
    ranges
}

/// The files the outlines of a task's context file name, as the card names them: the head line
/// of each outline under its `## Files` heading, a file not in the workspace yet left out.
#[must_use]
pub fn outlined(context: &str) -> Vec<String> {
    context
        .lines()
        .skip_while(|l| l.trim_end() != "## Files")
        .skip(1)
        .take_while(|l| !l.starts_with("## "))
        .filter(|l| !l.starts_with([' ', '*']))
        .filter_map(|l| {
            let (file, rest) = l.split_once(" (")?;
            rest.split_once(" lines)").filter(|(count, _)| count.parse::<usize>().is_ok()).map(|_| file.to_owned())
        })
        .collect()
}

/// The name an item declares: the word after its keyword, `hook` of `pub fn hook(event: &str)`,
/// `Worker` of `impl Worker`; `None` for an item with no word after the keyword.
#[must_use]
pub fn item_name(item: &str) -> Option<&str> {
    let mut rest = item.trim_start();
    if let Some(after) = rest.strip_prefix("pub") {
        rest = match after.strip_prefix('(') {
            Some(scoped) => scoped.split_once(')')?.1,
            None => after,
        }
        .trim_start();
    }
    let keyword = KEYWORDS.iter().filter(|k| rest.starts_with(*k)).max_by_key(|k| k.len())?;
    let name = rest[keyword.len()..].trim_start();
    let end = name.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(name.len());
    (end > 0).then(|| &name[..end])
}

/// The words a Rust item opens with, after any `pub`.
const KEYWORDS: [&str; 15] = [
    "fn",
    "async fn",
    "const fn",
    "unsafe fn",
    "struct",
    "enum",
    "union",
    "trait",
    "impl",
    "mod",
    "const",
    "static",
    "type",
    "macro_rules!",
    "unsafe impl",
];

/// A Rust item at the top level or one level in: the declaration without its body.
fn rust_item(line: &str) -> Option<String> {
    let indent = line.len() - line.trim_start().len();
    if indent > 4 {
        return None;
    }
    let mut rest = line.trim_start();
    if let Some(after) = rest.strip_prefix("pub") {
        rest = match after.strip_prefix('(') {
            Some(scoped) => scoped.split_once(')')?.1,
            None => after,
        }
        .trim_start();
    }
    let is_item = KEYWORDS.iter().any(|k| {
        rest.strip_prefix(k).is_some_and(|after| after.starts_with([' ', '<', '!']) || (k.ends_with('!') && after.starts_with(' ')))
    });
    if !is_item || rest.starts_with("mod tests") {
        return None;
    }
    // a declaration ends before its body, empty or not, or its `;`
    let shown = line.trim();
    let shown = shown.strip_suffix("{}").or_else(|| shown.strip_suffix("{ }")).unwrap_or(shown);
    Some(shown.trim_end_matches(['{', ';', ' ']).trim_end_matches(" where").to_owned())
}

/// A definition at the start of a line in a language the outline does not know.
fn generic_item(line: &str) -> Option<String> {
    const STARTS: [&str; 9] = ["function ", "def ", "class ", "export function ", "export class ", "export const ", "fn ", "func ", "sub "];
    STARTS.iter().any(|s| line.starts_with(s)).then(|| line.trim().trim_end_matches(['{', ':', ' ']).to_owned())
}

/// The outlines of the files a card names, under one heading, with the lines `changed` names
/// per file marked; a file that is not there is named as missing, so the card's list shows
/// its mistake.
pub fn outline_files(workspace: &Path, files: &[String], changed: &ChangedLines) -> String {
    let mut out = String::new();
    for file in files {
        match std::fs::read_to_string(workspace.join(file)) {
            Ok(text) => out.push_str(&outline_marked(Path::new(file), &text, changed.get(file).map_or(&[], Vec::as_slice))),
            Err(_) => {
                let _ = writeln!(out, "{file} (not in the workspace yet)");
            }
        }
    }
    out
}

/// The heading under which the untracked source files ride with the project context.
pub const UNTRACKED_HEADING: &str = "## Untracked sources";

/// How many untracked sources the list names before it counts the rest.
const UNTRACKED_ROOM: usize = 20;

/// The extensions an untracked file needs to count as a source.
const SOURCE_EXTENSIONS: [&str; 3] = ["rs", "py", "ps1"];

/// The source files git does not track yet, as `git ls-files --others --exclude-standard`
/// lists them, in its order. An outline only names the files a card names, and a card is
/// written from the tracked tree: a change that landed in a new file is invisible to both the
/// architect and the worker until it is committed, and this list is where it shows. With
/// `files`, the card's files, only the sources under the same crates (the first two components
/// of a card file's path, or its directory) are kept; empty `files` keeps them all. A tree
/// that is not a git repository, or git missing, has none.
#[must_use]
pub fn untracked_sources(workspace: &Path, files: &[String]) -> Vec<String> {
    let listed = std::process::Command::new("git")
        .args(["ls-files", "--others", "--exclude-standard"])
        .current_dir(workspace)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
        .unwrap_or_default();
    let crates: Vec<String> = files.iter().map(|f| crate_of(f)).collect();
    listed
        .lines()
        .map(|line| line.trim().trim_matches('"'))
        .filter(|path| Path::new(path).extension().and_then(|e| e.to_str()).is_some_and(|e| SOURCE_EXTENSIONS.contains(&e)))
        .filter(|path| crates.is_empty() || crates.iter().any(|c| c.is_empty() || path.starts_with(c.as_str())))
        .map(str::to_owned)
        .collect()
}

/// The crate a card file belongs to, as a path prefix: `crates/ra-tools/` of
/// `crates/ra-tools/src/lib.rs`; a file at the root or one level down has no crate and keeps
/// everything.
fn crate_of(file: &str) -> String {
    let parts: Vec<&str> = file.split('/').collect();
    if parts.len() < 3 {
        return String::new();
    }
    format!("{}/{}/", parts[0], parts[1])
}

/// The project context with the untracked sources after it, one per line under
/// [`UNTRACKED_HEADING`], the first [`UNTRACKED_ROOM`] named and the rest counted; the context
/// alone when there is none.
#[must_use]
pub fn with_untracked(project: &str, untracked: &[String]) -> String {
    if untracked.is_empty() {
        return project.to_owned();
    }
    let mut text = project.trim_end().to_owned();
    if !text.is_empty() {
        text.push_str("\n\n");
    }
    text.push_str(UNTRACKED_HEADING);
    text.push('\n');
    for file in &untracked[..untracked.len().min(UNTRACKED_ROOM)] {
        let _ = writeln!(text, "- {file}");
    }
    if untracked.len() > UNTRACKED_ROOM {
        let _ = writeln!(text, "… and {} more", untracked.len() - UNTRACKED_ROOM);
    }
    text
}

/// The changed line ranges of each path, 1-based and inclusive, keyed by the path as the diff
/// names it.
pub type ChangedLines = std::collections::HashMap<String, Vec<(usize, usize)>>;

/// The lines a `git diff -U0` touched, from its `+++` and `@@` headers: the new side of each
/// hunk; a pure deletion marks the line it happened after, so the item around it shows.
#[must_use]
pub fn changed_lines(diff: &str) -> ChangedLines {
    let mut changed = ChangedLines::new();
    let mut file: Option<String> = None;
    for line in diff.lines() {
        if let Some(path) = line.strip_prefix("+++ ") {
            let path = path.strip_prefix("b/").unwrap_or(path).trim_matches('"');
            file = (path != "/dev/null").then(|| path.to_owned());
        } else if let Some(hunk) = line.strip_prefix("@@ ")
            && let Some(file) = &file
            && let Some(added) = hunk.split(' ').find_map(|part| part.strip_prefix('+'))
        {
            let (start, count) = added.split_once(',').unwrap_or((added, "1"));
            let (Ok(start), Ok(count)) = (start.parse::<usize>(), count.parse::<usize>()) else { continue };
            let start = start.max(1);
            changed.entry(file.clone()).or_default().push((start, start + count.saturating_sub(1)));
        }
    }
    changed
}

/// What one run of a task left behind, for the next run to start from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handoff {
    /// When the run ended, as seconds since the Unix epoch.
    pub when: u64,
    pub model: String,
    /// `implemented`, `needs revision` or `failed`; a review that sends the card back retags it
    /// `needs revision (trivial)` or `needs revision (substantive)`.
    pub outcome: String,
    /// The check line: the command and whether it passed.
    pub check: String,
    /// The reviewer's verdict on the run, `accepted by <model>` or `needs revision (<weight>) ·
    /// <n> point(s) by <model>`; `None` until the reviewer judged it. A card is offered for
    /// acceptance only with one attached.
    pub verdict: Option<String>,
    /// The workspace-relative paths the run changed.
    pub changed: Vec<String>,
    /// The worker's report, clipped to its first lines.
    pub report: String,
    /// The clippy diagnostics the lint of the run found in the changed files, whole; empty when
    /// the lint was clean or did not run. Kept apart from the report so the clip never eats them.
    pub warnings: Vec<String>,
    /// The read set of a session that ended on its cap or a failed check, as [`read_set`] has
    /// it: `path` for a file read whole, `path:<from>-<to>` for a region. The next session opens
    /// on those regions instead of reading them in again; empty for any other run.
    pub read: Vec<String>,
    /// The failures of the run's failed check as [`failures_of`] reads them from the whole
    /// output, each its name line and its error lines; empty when the check passed. Kept apart
    /// from the report so the clip of the output never eats a failure.
    pub failures: Vec<String>,
}

/// The heading of a handoff's warnings in the file.
const WARNINGS_HEADING: &str = "### Warnings";

/// The heading of a handoff's failures in the file.
const FAILURES_HEADING: &str = "### Failures";

/// The mark on a failure's name line when the same failure comes out of the base the card's
/// branch started from: the card did not break it.
pub const ON_BASE: &str = " · also fails on the base";

/// How many failures one check keeps; past them a line counts the rest.
const FAILURES_KEPT: usize = 40;

/// How many error lines one failure keeps under its name.
const FAILURE_LINES: usize = 12;

/// The failures of a failed check's output, read by their structure rather than by where they
/// fall in it: every failing test of a cargo or a gradle run, each with the lines after its
/// `panicked at` or under its gradle name, then every compiler `error`, each with the lines up
/// to its blank line. When none of those is there, gradle's `What went wrong` block. Each
/// failure is its name line and its error lines, without blank lines; an output that names
/// nothing gives none.
#[must_use]
pub fn failures_of(output: &str) -> Vec<String> {
    let lines: Vec<&str> = output.lines().collect();
    let mut failures: Vec<(String, Vec<String>)> = Vec::new();
    // a test named twice, by its FAILED line and by its own output, is one failure with the
    // lines of the second
    let add = |failures: &mut Vec<(String, Vec<String>)>, name: String, detail: Vec<String>| match failures
        .iter_mut()
        .find(|(known, _)| *known == name)
    {
        Some((_, known)) if known.is_empty() => *known = detail,
        Some(_) => {}
        None => failures.push((name, detail)),
    };
    let mut at = 0;
    while at < lines.len() {
        let line = lines[at].trim_end();
        let bare = line.trim();
        if let Some(name) = cargo_failed(bare) {
            add(&mut failures, name.to_owned(), Vec::new());
        } else if let Some(name) = bare.strip_prefix("---- ").and_then(|rest| rest.strip_suffix(" stdout ----")) {
            // the test's own output: what it printed from its panic on
            let mut detail = Vec::new();
            let mut panicked = false;
            let mut next = at + 1;
            while next < lines.len() && !is_cargo_section_end(lines[next]) {
                let text = lines[next].trim_end();
                panicked |= text.contains("panicked at") || text.trim_start().starts_with("error");
                // the panic's message ends at its first blank line; what follows is not its
                let ended = panicked && text.trim().is_empty() && !detail.is_empty();
                if ended {
                    panicked = false;
                } else if panicked && !text.starts_with("note: run with") && detail.len() < FAILURE_LINES {
                    detail.push(text.to_owned());
                }
                next += 1;
            }
            add(&mut failures, name.trim().to_owned(), detail);
            at = next;
            continue;
        } else if let Some(name) = gradle_failed(line) {
            let mut detail = Vec::new();
            let mut next = at + 1;
            while next < lines.len() && lines[next].starts_with(char::is_whitespace) && !lines[next].trim().is_empty() {
                if detail.len() < FAILURE_LINES {
                    detail.push(lines[next].trim_end().to_owned());
                }
                next += 1;
            }
            add(&mut failures, name.to_owned(), detail);
            at = next;
            continue;
        } else if is_compiler_error(bare) {
            let mut detail = Vec::new();
            let mut next = at + 1;
            while next < lines.len() && !lines[next].trim().is_empty() && !is_compiler_error(lines[next].trim()) {
                if detail.len() < FAILURE_LINES {
                    detail.push(lines[next].trim_end().to_owned());
                }
                next += 1;
            }
            add(&mut failures, bare.to_owned(), detail);
            at = next;
            continue;
        }
        at += 1;
    }
    if failures.is_empty()
        && let Some(start) = lines.iter().position(|l| l.trim() == "* What went wrong:")
    {
        let detail: Vec<String> =
            lines[start + 1..].iter().take_while(|l| !l.trim().is_empty()).take(FAILURE_LINES).map(|l| l.trim_end().to_owned()).collect();
        if !detail.is_empty() {
            failures.push(("What went wrong:".to_owned(), detail));
        }
    }
    let total = failures.len();
    let mut kept: Vec<String> = failures
        .into_iter()
        .take(FAILURES_KEPT)
        .map(|(name, detail)| if detail.is_empty() { name } else { format!("{name}\n{}", detail.join("\n")) })
        .collect();
    if total > FAILURES_KEPT {
        kept.push(format!("… {} more failure(s) in the output", total - FAILURES_KEPT));
    }
    kept
}

/// The name of a cargo test `line` marks failed: `test name ... FAILED`, or `name --- FAILED`.
fn cargo_failed(line: &str) -> Option<&str> {
    let rest = line.strip_suffix(" FAILED")?;
    if let Some(name) = rest.strip_prefix("test ") {
        return Some(name.strip_suffix(" ...").unwrap_or(name).trim());
    }
    rest.strip_suffix(" ---").map(str::trim).filter(|name| !name.is_empty() && !name.contains(' '))
}

/// The name of a gradle test `line` marks failed: `Class > method FAILED`, at the line's start.
fn gradle_failed(line: &str) -> Option<&str> {
    let name = line.strip_suffix(" FAILED")?;
    (!line.starts_with(char::is_whitespace) && name.contains(" > ")).then_some(name.trim())
}

/// Whether a line of a test's own output ends it: the next test's, or cargo's list after them.
fn is_cargo_section_end(line: &str) -> bool {
    let line = line.trim();
    line.starts_with("---- ") || line == "failures:" || line.starts_with("test result:")
}

/// Whether `line` opens a compiler error: rustc's `error[E…]:` or `error:`, javac's `file:12:
/// error:`, kotlinc's `e: `; not cargo's summaries of them (`could not compile`, `test failed`).
fn is_compiler_error(line: &str) -> bool {
    const SUMMARIES: [&str; 4] = ["error: could not compile", "error: test failed", "error: aborting", "error: build failed"];
    if SUMMARIES.iter().any(|summary| line.starts_with(summary)) {
        return false;
    }
    line.starts_with("error[") || line.starts_with("error: ") || line.contains(": error: ") || line.starts_with("e: ")
}

/// `failures` with [`ON_BASE`] on the name line of each the base's own check also gave.
#[must_use]
pub fn marked_on_base(failures: &[String], base: &[String]) -> Vec<String> {
    let name = |failure: &str| failure.lines().next().unwrap_or_default().trim_end_matches(ON_BASE).to_owned();
    let on_base: Vec<String> = base.iter().map(|failure| name(failure)).collect();
    failures
        .iter()
        .map(|failure| {
            let (head, rest) = failure.split_once('\n').map_or((failure.as_str(), None), |(head, rest)| (head, Some(rest)));
            if head.ends_with(ON_BASE) || !on_base.contains(&name(failure)) {
                return failure.clone();
            }
            match rest {
                Some(rest) => format!("{head}{ON_BASE}\n{rest}"),
                None => format!("{head}{ON_BASE}"),
            }
        })
        .collect()
}

/// A task's context: the brief, the outlines and the handoffs, in `var/context/<id>.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskContext {
    pub id: i64,
    /// The card and its surroundings, as the run builds them.
    pub brief: String,
    /// The outlines of the files the card names, built afresh each run.
    pub outlines: String,
    /// The handoffs of the earlier runs, oldest first.
    pub handoffs: Vec<Handoff>,
    /// The latest `review by` block of the card's notes, verbatim, when the card is rerun after
    /// a reject; `None` for a fresh card. The notes are in the roadmap, which the worker does not
    /// read, so the prompt quotes them.
    pub review: Option<String>,
    /// The regions the last session read, as [`regions`] renders them, built afresh each run;
    /// the prompt opens with them. Not kept in the file: they are the files' current lines.
    pub regions: String,
}

/// The heading under which the last session's regions open the prompt.
pub const REGIONS_HEADING: &str = "## Read by the last session";

/// How many lines one region shows before it is clipped to an item boundary.
const REGION_LINES: usize = 200;

/// How many lines the regions show in all; the regions past it are named by their outline.
const REGIONS_ROOM: usize = 800;

/// How many items a clipped region names of the lines it leaves out.
const LEFT_OUT_SHOWN: usize = 8;

/// One entry of a read set: the path and the lines read of it, `None` for the whole file. A
/// path with a colon of its own (`D:/x.rs`) keeps it; only a `:<from>-<to>` tail is a range.
#[must_use]
pub fn read_entry(entry: &str) -> (&str, Option<(usize, usize)>) {
    let range = entry.rsplit_once(':').and_then(|(path, range)| {
        let (from, to) = range.split_once('-')?;
        Some((path, (from.parse().ok()?, to.parse().ok()?)))
    });
    match range {
        Some((path, range)) => (path, Some(range)),
        None => (entry, None),
    }
}

/// A file and its ranges, or `None` for a whole-file read, during the merge of read entries.
type FileRanges<'a> = (&'a str, Option<Vec<(usize, usize)>>);

/// The read set of a session from its reads in order, as the stream named them: one file after
/// another in the order first read, each file's ranges merged where they touch, a whole read
/// standing for the file.
#[must_use]
pub fn read_set(reads: &[String]) -> Vec<String> {
    let mut files: Vec<FileRanges> = Vec::new();
    for entry in reads {
        let (path, range) = read_entry(entry);
        let at = files.iter().position(|(p, _)| *p == path).unwrap_or_else(|| {
            files.push((path, Some(Vec::new())));
            files.len() - 1
        });
        match (&mut files[at].1, range) {
            (Some(ranges), Some(range)) => ranges.push(range),
            (held, None) => *held = None,
            (None, Some(_)) => {}
        }
    }
    let mut set = Vec::new();
    for (path, ranges) in files {
        let Some(mut ranges) = ranges else {
            set.push(path.to_owned());
            continue;
        };
        ranges.sort_unstable();
        let mut merged: Vec<(usize, usize)> = Vec::new();
        for (from, to) in ranges {
            match merged.last_mut() {
                Some(last) if from <= last.1 + 1 => last.1 = last.1.max(to),
                _ => merged.push((from, to)),
            }
        }
        set.extend(merged.into_iter().map(|(from, to)| format!("{path}:{from}-{to}")));
    }
    set
}

/// The regions of a read set as the next session reads them: per region its path and lines, the
/// outline of the items in it, and the lines themselves, numbered. A region over
/// [`REGION_LINES`] lines is clipped at the last item boundary the outline knows inside that
/// room, and the items it leaves out are named with their ranges; past [`REGIONS_ROOM`] lines
/// in all a region is named by its outline alone. A file no longer there is passed over.
#[must_use]
pub fn regions(workspace: &Path, read: &[String]) -> String {
    let mut out = String::new();
    let mut room = REGIONS_ROOM;
    for entry in read {
        let (file, range) = read_entry(entry);
        let path = if Path::new(file).is_absolute() { PathBuf::from(file) } else { workspace.join(file) };
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let lines: Vec<&str> = text.lines().collect();
        let (from, to) = range.unwrap_or((1, lines.len()));
        let (from, to) = (from.max(1), to.min(lines.len()));
        if from > to {
            continue;
        }
        let items = items(Path::new(file), &text);
        let shown_to = clip_region(&items, from, to, REGION_LINES.min(room));
        let _ = writeln!(out, "{file} lines {from}-{to} (of {})", lines.len());
        for (start, end, item) in items.iter().filter(|(start, end, _)| *start <= to && *end >= from) {
            let _ = writeln!(out, "  {start:>5}-{end:<5} {}", clip(item, 110));
        }
        if let Some(shown_to) = shown_to {
            for (number, line) in lines.iter().enumerate().take(shown_to).skip(from - 1) {
                let _ = writeln!(out, "{:>6}  {line}", number + 1);
            }
            room -= shown_to + 1 - from;
            if shown_to < to {
                let left: Vec<String> = items
                    .iter()
                    .filter(|(start, end, _)| *start <= to && *end > shown_to)
                    .take(LEFT_OUT_SHOWN)
                    .map(|(start, end, item)| format!("{} {start}-{end}", item_name(item).unwrap_or(item)))
                    .collect();
                let _ = write!(out, "     … lines {}-{to} left out", shown_to + 1);
                if !left.is_empty() {
                    let _ = write!(out, "; read them by range: {}", left.join(", "));
                }
                out.push('\n');
            }
        }
        out.push('\n');
    }
    out
}

/// The last line a region of `from..=to` shows in `room` lines: the whole region when it fits,
/// else the end of the last item that ends inside the room, else the room's last line; `None`
/// when there is no room left.
fn clip_region(items: &[(usize, usize, String)], from: usize, to: usize, room: usize) -> Option<usize> {
    if room == 0 {
        return None;
    }
    let last = from + room - 1;
    if to <= last {
        return Some(to);
    }
    Some(items.iter().map(|(_, end, _)| *end).filter(|end| (from..=last).contains(end)).max().unwrap_or(last))
}

/// The heading of the review in the file and the prompt.
pub const REVIEW_HEADING: &str = "## Review";

/// The latest reviewer's block at the head of a card's notes: its `review by` line and the
/// `- ` points under it, verbatim; `None` when the notes do not open with one.
#[must_use]
pub fn review_of(notes: &str) -> Option<String> {
    let mut lines = notes.lines().peekable();
    let first = lines.find(|l| l.starts_with("review by "))?;
    let mut block = vec![first];
    while let Some(point) = lines.next_if(|l| l.starts_with("- ")) {
        block.push(point);
    }
    Some(block.join("\n"))
}

impl TaskContext {
    /// The file of task `id` under `workspace`.
    #[must_use]
    pub fn path(workspace: &Path, id: i64) -> PathBuf {
        workspace.join(CONTEXT_DIR).join(format!("{id}.md"))
    }

    /// The context of task `id`: the handoffs of its file when there is one, nothing else yet;
    /// the run sets the brief and the outlines.
    #[must_use]
    pub fn open(workspace: &Path, id: i64) -> Self {
        let handoffs = std::fs::read_to_string(Self::path(workspace, id)).map(|text| parse_handoffs(&text)).unwrap_or_default();
        Self { id, brief: String::new(), outlines: String::new(), handoffs, review: None, regions: String::new() }
    }

    /// The text a run sends the worker: the regions the last session read, when it left any,
    /// then the brief, the outlines, and the last handoffs.
    #[must_use]
    pub fn prompt(&self) -> String {
        let mut text = String::new();
        if !self.regions.is_empty() {
            let _ = write!(
                text,
                "{REGIONS_HEADING}\nThe last session of this task read these regions; they are the files' current lines, so read only what they leave out.\n\n{}\n",
                self.regions.trim_end()
            );
        }
        text.push_str(&self.brief);
        if !self.outlines.is_empty() {
            text.push_str("\nFiles: read the regions the outline points at, not whole files.\n");
            text.push_str(&self.outlines);
        }
        if let Some(review) = &self.review {
            let _ = write!(
                text,
                "\n{REVIEW_HEADING}\nThe reviewer sent the last run back on these points; answer each:\n{}\n",
                review.trim_matches(['\n', '\r'])
            );
        }
        let shown = &self.handoffs[self.handoffs.len().saturating_sub(HANDOFFS_SHOWN)..];
        if !shown.is_empty() {
            text.push_str("\nEarlier runs of this task, oldest first; continue from the last one:\n");
            for handoff in shown {
                text.push_str(&handoff.render());
            }
        }
        text
    }

    /// The warnings of the last run: what its lint found, which the next run fixes first. Only
    /// the last run's count; an earlier run's warnings are stale once a later one came clean.
    #[must_use]
    pub fn last_warnings(&self) -> &[String] {
        self.handoffs.last().map_or(&[], |h| h.warnings.as_slice())
    }

    /// Adds the handoff of a run, keeping the newest [`HANDOFFS_KEPT`].
    pub fn record(&mut self, handoff: Handoff) {
        self.handoffs.push(handoff);
        let excess = self.handoffs.len().saturating_sub(HANDOFFS_KEPT);
        self.handoffs.drain(..excess);
    }

    /// Leaves a word from the run of card `from` on card `to`'s file, as a handoff from another
    /// card: the next run of `to` reads it with its own. Only the handoffs of the file survive
    /// this; the run of `to` sets the brief and the outlines afresh anyway.
    ///
    /// # Errors
    /// When the file cannot be written.
    pub fn leave_note(workspace: &Path, to: i64, from: i64, model: &str, when: u64, text: &str) -> anyhow::Result<()> {
        let mut task = Self::open(workspace, to);
        task.record(Handoff {
            when,
            model: model.to_owned(),
            outcome: format!("note from #{from}"),
            check: format!("no check: a word left by the run of #{from}"),
            verdict: None,
            changed: Vec::new(),
            report: text.to_owned(),
            warnings: Vec::new(),
            read: Vec::new(),
            failures: Vec::new(),
        });
        task.save(workspace)
    }

    /// Writes the file.
    ///
    /// # Errors
    /// When the directory or the file cannot be written.
    pub fn save(&self, workspace: &Path) -> anyhow::Result<()> {
        let path = Self::path(workspace, self.id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut text = format!("# Task #{} context\n\n## Brief\n\n{}\n", self.id, self.brief.trim_end());
        if !self.outlines.is_empty() {
            let _ = write!(text, "\n## Files\n\n{}", self.outlines);
        }
        // the review rides before the handoffs, where the reader of the handoffs skips it
        if let Some(review) = &self.review {
            let _ = write!(text, "\n{REVIEW_HEADING}\n\n{}\n", review.trim());
        }
        for handoff in &self.handoffs {
            text.push('\n');
            text.push_str(&handoff.render());
        }
        std::fs::write(&path, text).with_context(|| format!("cannot write {}", path.display()))
    }

    /// Removes the files of every task not in `keep`: the done cards. Returns how many went.
    #[must_use]
    pub fn prune(workspace: &Path, keep: &[i64]) -> usize {
        let Ok(entries) = std::fs::read_dir(workspace.join(CONTEXT_DIR)) else { return 0 };
        let mut removed = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            let id = path.file_stem().and_then(|s| s.to_str()).and_then(|s| s.parse::<i64>().ok());
            if let Some(id) = id
                && !keep.contains(&id)
                && std::fs::remove_file(&path).is_ok()
            {
                removed += 1;
            }
        }
        removed
    }
}

/// The mark of a failed check's fingerprint on a handoff's check line: ` · fingerprint <hex>`.
/// The line is kept in the file, so the next run of the card tells the same failure again.
pub const FINGERPRINT_MARK: &str = " · fingerprint ";

impl Handoff {
    /// The fingerprint of the failed check the run ended on, as its check line carries it;
    /// `None` for a run whose check passed or that predates the fingerprints.
    #[must_use]
    pub fn fingerprint(&self) -> Option<&str> {
        let (_, rest) = self.check.split_once(FINGERPRINT_MARK)?;
        let print = rest.split(" · ").next().unwrap_or_default().trim();
        (!print.is_empty()).then_some(print)
    }

    /// The handoff as a block of the file and of the prompt.
    fn render(&self) -> String {
        let mut text = format!("## Handoff {} · {} · {}\n", self.when, self.model, self.outcome);
        let _ = writeln!(text, "check: {}", self.check.trim());
        if let Some(verdict) = &self.verdict {
            let _ = writeln!(text, "verdict: {}", verdict.trim());
        }
        if !self.changed.is_empty() {
            let _ = writeln!(text, "changed: {}", self.changed.join(" "));
        }
        if !self.read.is_empty() {
            let _ = writeln!(text, "read: {}", self.read.join(" "));
        }
        let report: Vec<&str> = self
            .report
            .lines()
            .filter(|l| !l.starts_with("## Handoff ") && l.trim_end() != WARNINGS_HEADING && l.trim_end() != FAILURES_HEADING)
            .collect();
        let shown = &report[..report.len().min(REPORT_LINES)];
        if !shown.is_empty() {
            text.push_str(&shown.join("\n"));
            text.push('\n');
        }
        if report.len() > shown.len() {
            let _ = writeln!(text, "… {} more lines in the notes of the card", report.len() - shown.len());
        }
        // the failures and the warnings close the block, whole and after the clip, each a
        // paragraph
        if !self.failures.is_empty() {
            let _ = writeln!(text, "{FAILURES_HEADING}\n{}", self.failures.join("\n\n"));
        }
        if !self.warnings.is_empty() {
            let _ = writeln!(text, "{WARNINGS_HEADING}\n{}", self.warnings.join("\n\n"));
        }
        text
    }
}

/// The diagnostics of a warnings section, one per `warning`/`error` opening line; the blank
/// lines between them are not part of any.
fn parse_warnings(text: &str) -> Vec<String> {
    let mut warnings: Vec<String> = Vec::new();
    for line in text.lines() {
        if line.starts_with("warning") || line.starts_with("error") || warnings.is_empty() {
            if !line.trim().is_empty() {
                warnings.push(line.to_owned());
            }
        } else if let Some(last) = warnings.last_mut()
            && !line.trim().is_empty()
        {
            last.push('\n');
            last.push_str(line);
        }
    }
    warnings
}

/// The handoffs of a task's file, oldest first; a file without any gives none.
fn parse_handoffs(text: &str) -> Vec<Handoff> {
    let mut handoffs: Vec<Handoff> = Vec::new();
    // the warnings or the failures section of the current handoff, gathered to its end; the
    // flag says which
    let mut warnings: Option<String> = None;
    let mut in_failures = false;
    let close = |handoffs: &mut Vec<Handoff>, warnings: &mut Option<String>, in_failures: bool| {
        if let Some(text) = warnings.take()
            && let Some(current) = handoffs.last_mut()
        {
            if in_failures {
                current.failures = text.split("\n\n").map(str::trim).filter(|f| !f.is_empty()).map(str::to_owned).collect();
            } else {
                current.warnings = parse_warnings(&text);
            }
        }
    };
    for line in text.lines() {
        if let Some(header) = line.strip_prefix("## Handoff ") {
            close(&mut handoffs, &mut warnings, in_failures);
            let mut parts = header.split(" · ");
            let when = parts.next().and_then(|w| w.trim().parse().ok()).unwrap_or(0);
            let model = parts.next().unwrap_or("").trim().to_owned();
            let outcome = parts.next().unwrap_or("").trim().to_owned();
            handoffs.push(Handoff {
                when,
                model,
                outcome,
                check: String::new(),
                verdict: None,
                changed: Vec::new(),
                report: String::new(),
                warnings: Vec::new(),
                read: Vec::new(),
                failures: Vec::new(),
            });
            continue;
        }
        if handoffs.is_empty() {
            continue;
        }
        if line.starts_with("## ") {
            // another section after the handoffs; nothing of the format writes one, but a hand
            // edit may
            close(&mut handoffs, &mut warnings, in_failures);
            continue;
        }
        if line.trim_end() == WARNINGS_HEADING || line.trim_end() == FAILURES_HEADING {
            close(&mut handoffs, &mut warnings, in_failures);
            in_failures = line.trim_end() == FAILURES_HEADING;
            warnings = Some(String::new());
            continue;
        }
        let Some(current) = handoffs.last_mut() else { continue };
        if let Some(section) = &mut warnings {
            section.push_str(line);
            section.push('\n');
            continue;
        }
        if current.check.is_empty()
            && current.report.is_empty()
            && let Some(check) = line.strip_prefix("check: ")
        {
            check.clone_into(&mut current.check);
        } else if current.verdict.is_none()
            && current.report.is_empty()
            && let Some(verdict) = line.strip_prefix("verdict: ")
        {
            current.verdict = Some(verdict.to_owned());
        } else if current.changed.is_empty()
            && current.report.is_empty()
            && let Some(changed) = line.strip_prefix("changed: ")
        {
            current.changed = changed.split_whitespace().map(str::to_owned).collect();
        } else if current.read.is_empty()
            && current.report.is_empty()
            && let Some(read) = line.strip_prefix("read: ")
        {
            current.read = read.split_whitespace().map(str::to_owned).collect();
        } else if line.starts_with("… ") && line.ends_with("in the notes of the card") {
            // the count of the clipped lines is not part of the report
        } else {
            if !current.report.is_empty() {
                current.report.push('\n');
            }
            current.report.push_str(line);
        }
    }
    close(&mut handoffs, &mut warnings, in_failures);
    for handoff in &mut handoffs {
        handoff.report = handoff.report.trim().to_owned();
    }
    handoffs
}

/// The paths whose line of `git status --porcelain` differs between `before` and `after`: what
/// a run changed, whatever was already dirty.
#[must_use]
pub fn changed_between(before: &str, after: &str) -> Vec<String> {
    let before: std::collections::HashSet<&str> = before.lines().collect();
    let mut changed: Vec<String> = after
        .lines()
        .filter(|l| !before.contains(l))
        .filter_map(|l| l.get(3..))
        .map(|p| p.rsplit(" -> ").next().unwrap_or(p).trim_matches('"').to_owned())
        .collect();
    changed.sort();
    changed.dedup();
    changed
}

/// The prefix of a report line addressed to another card: `notes-for #<id>: …`.
const NOTES_FOR: &str = "notes-for #";

/// Splits the `notes-for #<id>: …` lines out of a worker's report: the report without them,
/// and the notes by card in the order the report first named each, several lines to one card
/// joined. A bullet mark before the prefix is allowed; a line without an id or a text stays in
/// the report, and a report without any comes back as it was.
#[must_use]
pub fn notes_for(report: &str) -> (String, Vec<(i64, String)>) {
    let mut kept: Vec<&str> = Vec::new();
    let mut notes: Vec<(i64, String)> = Vec::new();
    for line in report.lines() {
        let bare = line.trim().trim_start_matches(['-', '*', ' ']);
        let parsed = bare.get(..NOTES_FOR.len()).filter(|p| p.eq_ignore_ascii_case(NOTES_FOR)).and_then(|_| {
            let (id, text) = bare[NOTES_FOR.len()..].split_once(':')?;
            let id = id.trim().parse::<i64>().ok()?;
            let text = text.trim();
            (!text.is_empty()).then(|| (id, text.to_owned()))
        });
        match parsed {
            Some((id, text)) => match notes.iter_mut().find(|(to, _)| *to == id) {
                Some((_, held)) => {
                    held.push('\n');
                    held.push_str(&text);
                }
                None => notes.push((id, text)),
            },
            None => kept.push(line),
        }
    }
    if notes.is_empty() {
        return (report.to_owned(), notes);
    }
    let mut rest = kept.join("\n");
    if report.ends_with('\n') {
        rest.push('\n');
    }
    (rest, notes)
}

/// A text clipped to `room` characters with an ellipsis.
fn clip(text: &str, room: usize) -> String {
    if text.chars().count() <= room { text.to_owned() } else { format!("{}…", text.chars().take(room - 1).collect::<String>()) }
}

#[cfg(test)]
mod tests;
