//! The tools a bare model gets to act in a workspace. A harness such as Claude Code brings its
//! own; a model behind a chat endpoint gets these: the files, a search, and a short list of
//! commands. Every path stays inside the workspace, and every command comes off an allowlist.
//!
//! A tool never fails the run: whatever goes wrong comes back as text the model reads and acts
//! on, the way a person reads an error.

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use ra_providers::{ToolCall, ToolSpec};
use serde::Deserialize;
use serde_json::json;

/// The most characters a tool answer carries; a longer one is cut and says so.
const MAX_OUTPUT: usize = 16_000;
/// The most matches a search reports.
const MAX_HITS: usize = 200;
/// How long a command may run.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(600);
/// The directories a search and a listing skip.
const SKIPPED_DIRS: [&str; 4] = [".git", "target", "node_modules", "var"];

/// The tools over one workspace.
#[derive(Debug, Clone)]
pub struct Toolbox {
    root: PathBuf,
}

impl Toolbox {
    /// The toolbox over `root`, which must exist.
    ///
    /// # Errors
    /// When `root` is not a directory that can be resolved.
    pub fn new(root: &Path) -> anyhow::Result<Self> {
        let root = root.canonicalize().with_context(|| format!("no workspace at {}", root.display()))?;
        anyhow::ensure!(root.is_dir(), "the workspace {} is not a directory", root.display());
        Ok(Self { root })
    }

    /// The workspace.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The specs of every tool, for the request.
    pub fn specs() -> Vec<ToolSpec> {
        let spec = |name: &str, description: &str, properties: serde_json::Value, required: &[&str]| ToolSpec {
            name: name.to_owned(),
            description: description.to_owned(),
            parameters: json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false}),
        };
        vec![
            spec(
                "read_file",
                "Reads a text file of the workspace, with line numbers. Long files come in windows: pass offset (a 1-based line) and limit.",
                json!({
                    "path": {"type": "string", "description": "The path, relative to the workspace."},
                    "offset": {"type": "integer", "description": "The first line to read, 1-based; the start when left out."},
                    "limit": {"type": "integer", "description": "How many lines to read; 400 when left out."}
                }),
                &["path"],
            ),
            spec(
                "write_file",
                "Writes a whole file, creating it and its directories when they are new. For a change inside an existing file, prefer edit_file.",
                json!({
                    "path": {"type": "string", "description": "The path, relative to the workspace."},
                    "content": {"type": "string", "description": "The full new content."}
                }),
                &["path", "content"],
            ),
            spec(
                "edit_file",
                "Replaces one exact passage of a file with another. The old passage must occur exactly once; include enough surrounding lines to make it unique.",
                json!({
                    "path": {"type": "string", "description": "The path, relative to the workspace."},
                    "old": {"type": "string", "description": "The exact text to replace."},
                    "new": {"type": "string", "description": "The text that takes its place."}
                }),
                &["path", "old", "new"],
            ),
            spec(
                "list_dir",
                "Lists the entries of a directory of the workspace; directories end in a slash.",
                json!({"path": {"type": "string", "description": "The directory, relative to the workspace; the root when left out."}}),
                &[],
            ),
            spec(
                "search",
                "Finds the lines of the workspace's text files that contain a literal text, as path:line: text. Case-sensitive.",
                json!({
                    "text": {"type": "string", "description": "The literal text to find."},
                    "path": {"type": "string", "description": "The directory or file to search, relative to the workspace; everything when left out."}
                }),
                &["text"],
            ),
            spec(
                "run",
                "Runs one command in the workspace and returns its output and exit code. Allowed: cargo (any subcommand) and git status, diff, log, show. Arguments split on spaces; no shell, no quoting.",
                json!({"command": {"type": "string", "description": "The command line, such as `cargo test -p ra-roadmap`."}}),
                &["command"],
            ),
        ]
    }

    /// Executes one call and returns what the model reads: the result, or the reason it failed.
    pub async fn execute(&self, call: &ToolCall) -> String {
        match self.try_execute(call).await {
            Ok(text) => clip(&text),
            Err(e) => format!("error: {e:#}"),
        }
    }

    async fn try_execute(&self, call: &ToolCall) -> anyhow::Result<String> {
        let args: serde_json::Value = serde_json::from_str(&call.arguments).context("the arguments are not a JSON object")?;
        match call.name.as_str() {
            "read_file" => {
                let a: ReadArgs = parse(args)?;
                self.read_file(&a.path, a.offset.unwrap_or(1), a.limit.unwrap_or(400))
            }
            "write_file" => {
                let a: WriteArgs = parse(args)?;
                self.write_file(&a.path, &a.content)
            }
            "edit_file" => {
                let a: EditArgs = parse(args)?;
                self.edit_file(&a.path, &a.old, &a.new)
            }
            "list_dir" => {
                let a: ListArgs = parse(args)?;
                self.list_dir(a.path.as_deref().unwrap_or(""))
            }
            "search" => {
                let a: SearchArgs = parse(args)?;
                self.search(&a.text, a.path.as_deref().unwrap_or(""))
            }
            "run" => {
                let a: RunArgs = parse(args)?;
                self.run(&a.command).await
            }
            other => anyhow::bail!("no tool {other}; the tools are read_file, write_file, edit_file, list_dir, search and run"),
        }
    }

    /// A path of the workspace, resolved lexically and kept inside the root.
    fn resolve(&self, rel: &str) -> anyhow::Result<PathBuf> {
        let mut out = self.root.clone();
        for part in Path::new(rel).components() {
            match part {
                Component::CurDir => {}
                Component::Normal(name) => out.push(name),
                Component::ParentDir => {
                    anyhow::ensure!(out.pop() && out.starts_with(&self.root), "{rel} leaves the workspace");
                }
                Component::RootDir | Component::Prefix(_) => anyhow::bail!("{rel} is not relative to the workspace"),
            }
        }
        anyhow::ensure!(out.starts_with(&self.root), "{rel} leaves the workspace");
        Ok(out)
    }

    /// The path as the model named it, for the answers.
    fn display(&self, path: &Path) -> String {
        path.strip_prefix(&self.root).unwrap_or(path).to_string_lossy().replace('\\', "/")
    }

    fn read_file(&self, rel: &str, offset: usize, limit: usize) -> anyhow::Result<String> {
        let path = self.resolve(rel)?;
        let text = std::fs::read_to_string(&path).with_context(|| format!("cannot read {rel}"))?;
        let total = text.lines().count();
        let start = offset.max(1);
        let mut out = String::new();
        for (number, line) in text.lines().enumerate().skip(start - 1).take(limit.max(1)) {
            out.push_str(&format!("{:>5}│{line}\n", number + 1));
        }
        if out.is_empty() {
            out = format!("{rel} has {total} line(s); nothing from line {start}\n");
        } else if start - 1 + limit < total {
            out.push_str(&format!("… {total} lines in all; read on from line {}\n", start + limit));
        }
        Ok(out)
    }

    fn write_file(&self, rel: &str, content: &str) -> anyhow::Result<String> {
        let path = self.resolve(rel)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| format!("cannot create the directories of {rel}"))?;
        }
        let existed = path.exists();
        std::fs::write(&path, content).with_context(|| format!("cannot write {rel}"))?;
        Ok(format!("{} {} ({} bytes)", if existed { "replaced" } else { "wrote" }, self.display(&path), content.len()))
    }

    fn edit_file(&self, rel: &str, old: &str, new: &str) -> anyhow::Result<String> {
        anyhow::ensure!(!old.is_empty(), "old is empty; name the passage to replace");
        let path = self.resolve(rel)?;
        let text = std::fs::read_to_string(&path).with_context(|| format!("cannot read {rel}"))?;
        let count = text.matches(old).count();
        anyhow::ensure!(count != 0, "{rel} does not contain the old passage");
        anyhow::ensure!(count == 1, "{rel} contains the old passage {count} times; include more context to make it unique");
        let edited = text.replacen(old, new, 1);
        std::fs::write(&path, edited).with_context(|| format!("cannot write {rel}"))?;
        Ok(format!("edited {}", self.display(&path)))
    }

    fn list_dir(&self, rel: &str) -> anyhow::Result<String> {
        let path = self.resolve(rel)?;
        let mut names: Vec<String> = std::fs::read_dir(&path)
            .with_context(|| format!("cannot list {}", if rel.is_empty() { "the workspace" } else { rel }))?
            .filter_map(Result::ok)
            .map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                if entry.path().is_dir() { format!("{name}/") } else { name }
            })
            .collect();
        names.sort();
        Ok(if names.is_empty() { "(empty)".to_owned() } else { names.join("\n") })
    }

    fn search(&self, needle: &str, rel: &str) -> anyhow::Result<String> {
        anyhow::ensure!(!needle.is_empty(), "text is empty");
        let start = self.resolve(rel)?;
        let mut hits: Vec<String> = Vec::new();
        let mut stack = vec![start];
        while let Some(path) = stack.pop() {
            if hits.len() >= MAX_HITS {
                break;
            }
            if path.is_dir() {
                let mut entries: Vec<PathBuf> = std::fs::read_dir(&path)
                    .with_context(|| format!("cannot read {}", self.display(&path)))?
                    .filter_map(Result::ok)
                    .map(|e| e.path())
                    .filter(|p| !p.file_name().is_some_and(|n| SKIPPED_DIRS.iter().any(|s| n == *s)))
                    .collect();
                // reverse, so the stack pops in name order
                entries.sort();
                entries.reverse();
                stack.extend(entries);
                continue;
            }
            // a binary file is not text; skip it quietly
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let shown = self.display(&path);
            for (number, line) in text.lines().enumerate() {
                if line.contains(needle) {
                    hits.push(format!("{shown}:{}: {}", number + 1, line.trim()));
                    if hits.len() >= MAX_HITS {
                        break;
                    }
                }
            }
        }
        Ok(match hits.len() {
            0 => format!("no line contains {needle:?}"),
            n if n >= MAX_HITS => format!("{}\n… the first {MAX_HITS} matches only; narrow the search", hits.join("\n")),
            _ => hits.join("\n"),
        })
    }

    async fn run(&self, command: &str) -> anyhow::Result<String> {
        let mut parts = command.split_whitespace();
        let program = parts.next().context("the command is empty")?;
        let args: Vec<&str> = parts.collect();
        anyhow::ensure!(allowed(program, &args), "{program} is not allowed; the commands are cargo, and git status, diff, log and show");
        let child = tokio::process::Command::new(program).args(&args).current_dir(&self.root).stdin(std::process::Stdio::null()).output();
        let output = tokio::time::timeout(COMMAND_TIMEOUT, child)
            .await
            .map_err(|_| anyhow::anyhow!("{program} ran past {} seconds and was stopped", COMMAND_TIMEOUT.as_secs()))?
            .with_context(|| format!("cannot start {program}"))?;
        let mut text = String::new();
        text.push_str(&String::from_utf8_lossy(&output.stdout));
        if !output.stderr.is_empty() {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&String::from_utf8_lossy(&output.stderr));
        }
        if !text.ends_with('\n') && !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&match output.status.code() {
            Some(code) => format!("exit code {code}"),
            None => "stopped by a signal".to_owned(),
        });
        Ok(text)
    }
}

/// Whether a command is on the allowlist: any cargo subcommand, and the git commands that only
/// read the repository.
fn allowed(program: &str, args: &[&str]) -> bool {
    match program {
        "cargo" => true,
        "git" => matches!(args.first().copied(), Some("status" | "diff" | "log" | "show")),
        _ => false,
    }
}

/// A tool answer within the limit; a long one keeps its head and its tail, since a build log
/// opens with the first error and closes with the verdict.
fn clip(text: &str) -> String {
    if text.chars().count() <= MAX_OUTPUT {
        return text.to_owned();
    }
    let head: String = text.chars().take(MAX_OUTPUT / 2).collect();
    let tail: String = text.chars().rev().take(MAX_OUTPUT / 2).collect::<Vec<_>>().into_iter().rev().collect();
    format!("{head}\n… {} characters cut …\n{tail}", text.chars().count() - MAX_OUTPUT)
}

fn parse<T: for<'de> Deserialize<'de>>(args: serde_json::Value) -> anyhow::Result<T> {
    serde_json::from_value(args).context("the arguments do not fit the tool")
}

#[derive(Deserialize)]
struct ReadArgs {
    path: String,
    offset: Option<usize>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct WriteArgs {
    path: String,
    content: String,
}

#[derive(Deserialize)]
struct EditArgs {
    path: String,
    old: String,
    new: String,
}

#[derive(Deserialize)]
struct ListArgs {
    path: Option<String>,
}

#[derive(Deserialize)]
struct SearchArgs {
    text: String,
    path: Option<String>,
}

#[derive(Deserialize)]
struct RunArgs {
    command: String,
}

#[cfg(test)]
mod tests;
