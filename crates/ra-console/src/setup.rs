//! A directory without a roadmap. `rc`, the console and the window treat the directory they
//! start in as the project; when it has no `rhipper.md`, nothing writes one on the quiet. The
//! person chooses: a roadmap drafted by a strong model from what the directory shows (the
//! README, the layout, the manifest), a roadmap-shaped file already there (ROADMAP.md, TODO.md,
//! a plan under docs/) converted by the same model, or an empty one. The model answers with the
//! plain [`ra_roadmap::outline`], which goes into the store as it is; the person edits after.

use std::path::{Path, PathBuf};

use anyhow::Context;
use ra_providers::{CompletionRequest, Message};
use ra_roadmap::model::GambitState;
use ra_roadmap::store::Store;
use ra_roadmap::{markdown, outline};

use crate::agent::{self, Worker};

/// The roadmap file a project keeps.
pub const ROADMAP_FILE: &str = "rhipper.md";

/// The model that drafts a roadmap: the strongest, once per project.
pub const SETUP_MODEL: &str = "claude-opus-5-5";

/// Where a model's answer goes when it does not parse, so nothing is lost.
const DRAFT_FILE: &str = "var/setup-draft.md";

/// How the roadmap comes to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// A root project and nothing else.
    Empty,
    /// The model drafts it from the directory.
    Draft,
    /// The model converts this file.
    Import(PathBuf),
}

/// Whether `root` has a roadmap.
pub fn exists(root: &Path) -> bool {
    root.join(ROADMAP_FILE).is_file()
}

/// The roadmap-shaped files of `root`: a plan, a todo, a backlog, at the top or under the
/// documentation directories, by name.
pub fn candidates(root: &Path) -> Vec<PathBuf> {
    const NAMES: [&str; 8] = ["roadmap", "todo", "plan", "tasks", "backlog", "milestones", "project", "spec"];
    const DIRS: [&str; 5] = ["", "docs", "doc", "planning", ".github"];
    let mut found = Vec::new();
    for dir in DIRS {
        let Ok(entries) = std::fs::read_dir(root.join(dir)) else { continue };
        let mut here: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .filter(|p| {
                let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
                (name.ends_with(".md") || name.ends_with(".txt")) && NAMES.iter().any(|n| name.starts_with(n))
            })
            .filter_map(|p| p.strip_prefix(root).ok().map(Path::to_path_buf))
            .collect();
        here.sort();
        found.extend(here);
    }
    found
}

/// The lines that tell a person what to do about a missing roadmap, with the candidates.
pub fn hint(root: &Path) -> Vec<String> {
    let shown = std::fs::canonicalize(root).map_or_else(|_| root.display().to_string(), |p| p.display().to_string().replace("\\\\?\\", ""));
    let mut lines = vec![
        format!("no {ROADMAP_FILE} in {shown}: this directory is not a Rhipper project yet"),
        format!("  rc init                  drafts a roadmap with {SETUP_MODEL} from the README, the layout and the manifest"),
        "  rc init --import <file>  converts a roadmap-shaped file into one, keeping every item".to_owned(),
        "  rc init --empty          starts with the bare project".to_owned(),
    ];
    let found = candidates(root);
    if !found.is_empty() {
        lines.push(format!(
            "  found here: {}",
            found.iter().map(|p| p.display().to_string().replace('\\', "/")).collect::<Vec<_>>().join(", ")
        ));
    }
    lines
}

/// What the model reads to draft a roadmap: the project context's draft with the person's
/// standing direction after it when there is one, the head of the README, the layout two levels
/// deep with the file counts, and the headings of the documents.
#[must_use]
pub fn brief(root: &Path, direction: &[String]) -> String {
    let mut text = ra_context::with_direction(&ra_context::Project::draft(root), direction);
    if let Some(readme) = ["README.md", "README", "readme.md"].iter().find_map(|n| std::fs::read_to_string(root.join(n)).ok()) {
        let head: Vec<&str> = readme.lines().take(150).collect();
        text.push_str("\n## README\n\n");
        text.push_str(&head.join("\n"));
        text.push('\n');
    }
    text.push_str("\n## Layout\n\n");
    text.push_str(&layout(root));
    let docs = doc_headings(root);
    if !docs.is_empty() {
        text.push_str("\n## Documents\n\n");
        text.push_str(&docs);
    }
    text
}

/// The directories two levels deep with how many files of which kinds they hold.
fn layout(root: &Path) -> String {
    const SKIPPED: [&str; 6] = [".git", "target", "node_modules", "var", ".idea", "dist"];
    let mut out = String::new();
    let walk = |dir: &Path, depth: usize, out: &mut String| {
        let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
        let mut dirs = Vec::new();
        let mut kinds: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                if !SKIPPED.contains(&name.as_str()) && !name.starts_with('.') {
                    dirs.push(path);
                }
            } else {
                let ext = path.extension().map_or_else(|| "(none)".to_owned(), |e| e.to_string_lossy().into_owned());
                *kinds.entry(ext).or_default() += 1;
            }
        }
        let shown = dir.strip_prefix(root).map_or_else(|_| ".".to_owned(), |p| p.display().to_string().replace('\\', "/"));
        let counts: Vec<String> = kinds.iter().map(|(k, n)| format!("{n} {k}")).collect();
        out.push_str(&format!("{}{}/ — {}\n", "  ".repeat(depth), if shown.is_empty() { "." } else { &shown }, counts.join(", ")));
        dirs.sort();
        dirs
    };
    let first = walk(root, 0, &mut out);
    for dir in first.iter().take(40) {
        let second = walk(dir, 1, &mut out);
        for sub in second.iter().take(20) {
            walk(sub, 2, &mut out);
        }
    }
    out
}

/// The headings of the Markdown files at the top and under docs/, so the model sees what has
/// been written down.
fn doc_headings(root: &Path) -> String {
    let mut out = String::new();
    for dir in ["", "docs", "doc"] {
        let Ok(entries) = std::fs::read_dir(root.join(dir)) else { continue };
        let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "md")).collect();
        files.sort();
        for file in files.iter().take(20) {
            let Ok(text) = std::fs::read_to_string(file) else { continue };
            let headings: Vec<&str> = text.lines().filter(|l| l.starts_with('#')).take(30).collect();
            if headings.is_empty() {
                continue;
            }
            out.push_str(&format!("{}:\n", file.strip_prefix(root).unwrap_or(file).display().to_string().replace('\\', "/")));
            for h in headings {
                out.push_str(&format!("  {h}\n"));
            }
        }
    }
    out
}

/// Makes the roadmap of `root` the way `mode` says, and says what happened, one line each.
///
/// # Errors
/// When a roadmap is already there, the model cannot be reached, its outline does not parse
/// (the text is kept in `var/setup-draft.md`), or the file cannot be written.
pub async fn init(root: &Path, worker: &Worker, mode: Mode) -> anyhow::Result<Vec<String>> {
    anyhow::ensure!(!exists(root), "{ROADMAP_FILE} is already here; edit it, or move it away first");
    let path = root.join(ROADMAP_FILE);
    let fallback = std::fs::canonicalize(root)
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "Project".to_owned());
    let mut lines = Vec::new();
    match mode {
        Mode::Empty => {
            Store::open(&path, &fallback).await?;
            lines.push(format!("{ROADMAP_FILE} written with the project {fallback} and nothing else; `rc add` fills it"));
        }
        Mode::Draft | Mode::Import(_) => {
            let import = match &mode {
                Mode::Import(file) => {
                    Some(std::fs::read_to_string(root.join(file)).with_context(|| format!("cannot read {}", file.display()))?)
                }
                Mode::Draft | Mode::Empty => None,
            };
            // a document that is a roadmap file brings its gambits: they steer the draft and
            // stand in the new file
            let gambits = import.as_deref().and_then(|text| markdown::parse_file(text).ok()).map(|r| r.gambits).unwrap_or_default();
            let answer = outline_from_model(worker, root, import.as_deref(), &agent::direction_lines(&gambits)).await?;
            // the one answer carries the outline and, after the marker, the project context
            let (text, context) = answer.split_once(CONTEXT_MARK).map_or((answer.as_str(), None), |(o, c)| (o, Some(c.trim())));
            let parsed = match outline::parse(text) {
                Ok(parsed) => parsed,
                Err(e) => {
                    let _ = std::fs::create_dir_all(root.join("var"));
                    let _ = std::fs::write(root.join(DRAFT_FILE), &answer);
                    anyhow::bail!("the model's outline did not parse ({e:#}); its text is in {DRAFT_FILE}, fix it and `rc import` it");
                }
            };
            let store = Store::open(&path, parsed.title.as_deref().unwrap_or(&fallback)).await?;
            for gambit in &gambits {
                let id = store.add_gambit(&gambit.text, gambit.weight, &gambit.focus).await?;
                if gambit.state != GambitState::Active {
                    store.set_gambit_state(id, gambit.state).await?;
                }
            }
            let ids = store.import(1, &parsed).await?;
            lines.push(format!(
                "{ROADMAP_FILE} written: {} ({} cards) {} by {}",
                parsed.summary(),
                ids.len(),
                if matches!(mode, Mode::Draft) { "drafted" } else { "imported" },
                worker.model
            ));
            lines.push("review it with `rc show`, edit the file by hand, and `rc run <id>` the first card".to_owned());
            if let Some(context) = context.filter(|c| c.lines().count() >= 4) {
                std::fs::write(root.join(ra_context::PROJECT_FILE), format!("{context}\n"))
                    .with_context(|| format!("cannot write {}", ra_context::PROJECT_FILE))?;
                lines.push(format!(
                    "{} written by {}; every run reads it, edit it as the project moves",
                    ra_context::PROJECT_FILE,
                    worker.model
                ));
            }
        }
    }
    // the project context goes with the roadmap: the model's when it wrote one, else a draft
    // from the directory
    if !root.join(ra_context::PROJECT_FILE).exists()
        && let Ok(text) = ra_context::Project::ensure(root)
        && !text.is_empty()
    {
        lines.push(format!("{} drafted from the directory; edit it by hand, every run reads it", ra_context::PROJECT_FILE));
    }
    Ok(lines)
}

/// The line that parts the outline from the project context in the model's answer.
const CONTEXT_MARK: &str = "---context---";

/// One chat completion that answers with the outline: the brief of the directory with the
/// standing direction, and the file to convert when there is one.
async fn outline_from_model(worker: &Worker, root: &Path, import: Option<&str>, direction: &[String]) -> anyhow::Result<String> {
    let provider = worker.provider()?;
    let brief = brief(root, direction);
    let user = match import {
        Some(text) => format!(
            "Convert the document below into the outline, keeping every item and its order; group a flat list under milestones you name; an item the document marks as done gets [done]. The project, for context:\n\n{brief}\n\n## The document to convert\n\n{text}"
        ),
        None => format!(
            "Draft the roadmap of this project from what follows. Three to six milestones, four to ten tasks each, the first milestone the work that unblocks the rest; name the files a task lands in when the layout shows them.\n\n{brief}"
        ),
    };
    let answer = provider
        .complete(CompletionRequest {
            model: worker.model.clone(),
            messages: vec![Message::system(ARCHITECT), Message::user(user)],
            temperature: Some(0.3),
            max_tokens: Some(8000),
            tools: Vec::new(),
            workspace: None,
            budget_usd: None,
            resume: None,
            max_turns: None,
            max_context: None,
            effort: None,
        })
        .await
        .map_err(agent::map_error)?;
    worker.record(None, &answer.usage, answer.turns).await;
    Ok(answer.text)
}

/// The system prompt of a draft: the outline, the marker, the project context, nothing else.
const ARCHITECT: &str = "You are the architect of a roadmap-driven development orchestrator. You answer with two parts and nothing else. First the roadmap as a plain outline: a `# <project title>` line; `## <milestone> {cN}` lines, each followed by an `objective:` line and an `acceptance:` line; under each milestone `- <task> {cN}` lines, each followed by indented `objective:`, `acceptance:` and, when the files are known, `files:` lines (workspace-relative paths separated by spaces). `{cN}` is the complexity from 1 (an hour) to 5 (days). An objective says what the task must achieve, one topic per sentence, and names no work already done. An acceptance is checkable by a person or a command: a command in backticks when there is one. Then a line that is exactly `---context---`, then the project context: the Markdown file every worker run of this project reads first, under sixty lines: `# <project title>` and one paragraph of what the project is; `## Layout`, one line per directory or crate as the brief shows them, what each holds; `## Build and checks`, the commands that build, test and lint, from the manifest and the README, and the check a card runs; `## Conventions`, what the code shows (language and edition, where tests live, naming, formatting) and these standing ones: read the files the card names first, by the regions the outline points at; search before exploring; run the check before reporting; report what changed and what was verified, briefly. No prose outside the two parts, no code fence.";

#[cfg(test)]
mod tests;
