//! The Claude Code adapter: one headless `claude -p` run per completion, on the machine's own
//! Claude login, so a subscription pays for the work and no API key is involved.
//!
//! Claude Code is a harness, not a bare model: it brings its own tools. A request with a
//! workspace lets it act there with its file tools and a short list of shell commands; a request
//! without one runs with no tools and one turn, a plain chat completion. The `tools` of the
//! request are not forwarded: the harness owns its tool set, and the answer carries no tool
//! calls. `temperature` and `max_tokens` have no flag on the command line and are not sent.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use ra_gotchas::{Catalogue, Conditions, Hit, Journal, Verdict, Where};
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

use crate::{Completion, CompletionFuture, CompletionRequest, Message, ModelProvider, ProviderError, Usage};

/// Where the plan's limits are kept, beside the roadmap.
pub const LIMITS_FILE: &str = "var/limits.json";

/// The built-in tools a worker gets: the files, the search, Bash, the web, the plan and the
/// subagents; not PowerShell, whose commands the allowlist cannot read on this platform.
const WORKER_TOOLS: &str = "Bash,Read,Edit,Write,Glob,Grep,TodoWrite,Task,WebFetch,WebSearch,NotebookEdit";

/// The Claude Code provider.
pub struct ClaudeCode {
    /// The command; `claude` on the path unless told otherwise.
    binary: PathBuf,
    /// The most turns a worker run may take before the harness stops it.
    max_turns: u32,
    /// The shell commands a worker may run, as Claude Code permission rules such as `Bash(cargo *)`.
    allowed_shell: Vec<String>,
    /// The effort the harness asks of the model: `medium` unless told otherwise, the efficient
    /// setting for the routine card; a hard one can ask for more.
    effort: String,
    /// The `rc` binary the harness's tool hooks call, when the gotchas guard a run.
    hooks: Option<PathBuf>,
    /// The gotchas the stream is read against, when there is a catalogue.
    catalogue: Option<Catalogue>,
}

impl Default for ClaudeCode {
    fn default() -> Self {
        Self::new()
    }
}

impl ClaudeCode {
    /// The provider over the `claude` on the path, with a worker allowed to build, test and read
    /// the git state.
    pub fn new() -> Self {
        Self {
            binary: PathBuf::from("claude"),
            max_turns: 60,
            // the build, the read-only git commands, and the read-only shell: a denied `ls` costs a
            // turn and teaches nothing. Bash only: the PowerShell tool wraps every command in a
            // prelude no rule can match, so the worker does not get it at all
            allowed_shell: [
                "Bash(cargo *)",
                "Bash(git status*)",
                "Bash(git diff*)",
                "Bash(git log*)",
                "Bash(git show*)",
                "Bash(ls*)",
                "Bash(cat *)",
                "Bash(head *)",
                "Bash(tail *)",
                "Bash(wc *)",
                "Bash(grep *)",
                "Bash(rg *)",
                "Bash(find *)",
                "Bash(pwd*)",
                "Bash(tree*)",
                "Bash(mkdir *)",
            ]
            .map(str::to_owned)
            .to_vec(),
            effort: "medium".to_owned(),
            hooks: None,
            catalogue: None,
        }
    }

    /// The effort level: `low`, `medium`, `high`, `xhigh` or `max`.
    pub fn with_effort(mut self, effort: impl Into<String>) -> Self {
        self.effort = effort.into();
        self
    }

    /// Runs another binary, for a pinned install or a test double.
    pub fn with_binary(mut self, binary: impl Into<PathBuf>) -> Self {
        self.binary = binary.into();
        self
    }

    /// Routes the harness's tool hooks to `rc hook pre|post` at `rc`, so the gotchas act before
    /// a command runs and right after a result, and reads the stream against `catalogue` to
    /// count what the run hit.
    pub fn with_gotchas(mut self, rc: impl Into<PathBuf>, catalogue: Catalogue) -> Self {
        self.hooks = Some(rc.into());
        self.catalogue = Some(catalogue);
        self
    }

    /// The most turns a worker run may take.
    pub fn with_max_turns(mut self, turns: u32) -> Self {
        self.max_turns = turns;
        self
    }

    /// The permission rules a worker gets.
    #[cfg(test)]
    fn new_allowed(&self) -> &[String] {
        &self.allowed_shell
    }

    /// The command-line arguments of one request. The prompt goes on stdin, so its length is no
    /// concern of the argument list.
    fn args(&self, request: &CompletionRequest) -> Vec<String> {
        // not --bare: it skips the hooks and plugins, but it also skips the OAuth login the
        // subscription route runs on (card #11 found out)
        // stream-json: the events arrive as the harness works, so the console can show the work
        let mut args = ["-p", "--output-format", "stream-json", "--verbose"].map(str::to_owned).to_vec();
        if request.workspace.is_none() {
            // a chat completion leaves no session behind; a worker keeps its own, for the next
            // run of the task to resume
            args.push("--no-session-persistence".to_owned());
        }
        args.extend(["--model".to_owned(), request.model.clone(), "--effort".to_owned(), self.effort.clone()]);
        if let Some(session) = request.resume.as_deref().filter(|s| !s.trim().is_empty()) {
            // the harness picks up where the last run of the task left off, its context intact
            args.extend(["--resume".to_owned(), session.trim().to_owned()]);
        }
        let system = system_prompt(&request.messages);
        if !system.is_empty() {
            args.extend(["--system-prompt".to_owned(), system]);
        }
        if request.workspace.is_some() {
            // a worker: the file tools and the allowed shell commands, inside the workspace
            args.extend(["--permission-mode".to_owned(), "acceptEdits".to_owned()]);
            args.extend(["--max-turns".to_owned(), self.max_turns.to_string()]);
            if !self.allowed_shell.is_empty() {
                args.push("--allowedTools".to_owned());
                args.extend(self.allowed_shell.iter().cloned());
            }
            // the built-in tools without PowerShell: every one of its commands is refused here,
            // and a worker that keeps trying it spends its turns on nothing. A denied tool is
            // still offered; a tool left off the list is not there to try
            args.extend(["--tools".to_owned(), WORKER_TOOLS.to_owned(), "--disallowedTools".to_owned(), "PowerShell".to_owned()]);
            if let Some(rc) = &self.hooks {
                args.extend(["--settings".to_owned(), hook_settings(rc)]);
            }
        } else {
            // a chat completion: no tools, one turn
            args.extend(["--tools".to_owned(), String::new(), "--max-turns".to_owned(), "1".to_owned()]);
        }
        if let Some(budget) = request.budget_usd {
            // the harness stops the run itself once it has spent this much
            args.extend(["--max-budget-usd".to_owned(), format!("{budget:.2}")]);
        }
        args
    }

    async fn run(&self, request: CompletionRequest) -> Result<Completion, ProviderError> {
        let prompt = prompt_of(&request.messages);
        let mut command = tokio::process::Command::new(&self.binary);
        command.args(self.args(&request)).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        if let Some(dir) = &request.workspace {
            command.current_dir(dir);
        }
        // a nested session refuses to start; the console may itself run under Claude Code
        command.env_remove("CLAUDECODE");
        let mut child = command.spawn().map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                ProviderError::Unavailable(format!("{} is not installed or not on the path", self.binary.display()))
            }
            _ => ProviderError::Unavailable(format!("cannot start {}: {e}", self.binary.display())),
        })?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(prompt.as_bytes()).await.map_err(|e| ProviderError::Other(format!("cannot write the prompt: {e}")))?;
            drop(stdin);
        }
        // stderr drains on its own task, so a chatty harness cannot fill the pipe and stall
        let stderr_task = child.stderr.take().map(|mut pipe| {
            tokio::spawn(async move {
                let mut text = String::new();
                let _ = tokio::io::AsyncReadExt::read_to_string(&mut pipe, &mut text).await;
                text
            })
        });
        // the events, one JSON object per line: the work goes to the progress trace as it
        // happens, and the last `result` event is the answer
        let mut result: Option<String> = None;
        let mut last_line = String::new();
        let mut gotchas: Vec<Hit> = Vec::new();
        // which tool each call id belongs to, so only a shell's output is read for gotchas
        let mut tools: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        // a console holds the binary while var/console.pid is in the workspace
        let conditions = Conditions { console_running: request.workspace.as_deref().is_some_and(|w| w.join("var/console.pid").exists()) };
        if let Some(pipe) = child.stdout.take() {
            let mut lines = tokio::io::BufReader::new(pipe).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                for step in progress_of(&line, request.workspace.as_deref()) {
                    tracing::info!(target: "ra::progress", "{step}");
                }
                if let Some(catalogue) = &self.catalogue {
                    // where the hit showed, from the event that carried it
                    let place = if line.contains(r#""type":"assistant""#) {
                        Where::Command
                    } else if line.contains(r#""type":"user""#) {
                        Where::Result
                    } else {
                        Where::Answer
                    };
                    for hit in gotchas_of_event(catalogue, &line, conditions, &mut tools) {
                        tracing::info!(target: "ra::progress", "⚠ {}: {}", hit.id, hit.excerpt);
                        // the journal is written here, as the hits come, so a run that ends badly
                        // still counts them
                        if let Some(workspace) = &request.workspace {
                            if let Err(e) = Journal::at(&workspace.join("var/gotchas.log")).record(None, place, &hit) {
                                tracing::warn!("cannot write the gotchas journal: {e:#}");
                            }
                        }
                        gotchas.push(hit);
                    }
                }
                if line.contains(r#""type":"result""#) {
                    result = Some(line.clone());
                }
                // the plan's windows, kept for the console and the window to show
                if let Some(newer) = crate::limits::Limits::from_event(&line) {
                    let path = request.workspace.clone().unwrap_or_default().join(LIMITS_FILE);
                    let mut limits = crate::limits::Limits::load(&path);
                    limits.merge(newer);
                    if let Err(e) = limits.save(&path) {
                        tracing::warn!("cannot write the limits: {e:#}");
                    }
                }
                if !line.trim().is_empty() {
                    last_line = line;
                }
            }
        }
        let status = child.wait().await.map_err(|e| ProviderError::Other(format!("claude did not finish: {e}")))?;
        let stderr = match stderr_task {
            Some(task) => task.await.unwrap_or_default(),
            None => String::new(),
        };
        match result {
            // a result names the reason itself, exit code or not
            Some(result) => parse_result(&result).map(|mut completion| {
                completion.gotchas = gotchas;
                completion
            }),
            None => {
                let reason = stderr.lines().map(str::trim).filter(|l| !l.is_empty()).next_back().unwrap_or(last_line.trim());
                let reason = if reason.is_empty() { "no output" } else { reason };
                Err(ProviderError::Other(format!("claude exited with {status} and no result: {}", snippet(reason))))
            }
        }
    }
}

/// The steps of the work one stream event shows, as short lines: a tool call as its name and
/// its target, a text block as its first line. Thinking stays out; it is not shown and not ours.
fn progress_of(line: &str, workspace: Option<&Path>) -> Vec<String> {
    let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else { return Vec::new() };
    if event.get("type").and_then(serde_json::Value::as_str) != Some("assistant") {
        return Vec::new();
    }
    let Some(blocks) = event.pointer("/message/content").and_then(serde_json::Value::as_array) else { return Vec::new() };
    blocks
        .iter()
        .filter_map(|block| match block.get("type").and_then(serde_json::Value::as_str) {
            Some("tool_use") => {
                let name = block.get("name").and_then(serde_json::Value::as_str).unwrap_or("tool");
                Some(describe_call(name, block.get("input").unwrap_or(&serde_json::Value::Null), workspace))
            }
            Some("text") => {
                let text = block.get("text").and_then(serde_json::Value::as_str)?.trim();
                let first = text.lines().find(|l| !l.trim().is_empty())?.trim();
                Some(format!("… {}", clip_line(first, 140)))
            }
            _ => None,
        })
        .collect()
}

/// One tool call as a line: `$ cargo test` for a command, `Read src/main.rs` for a file tool,
/// `Grep pattern` for a search; a path inside the workspace is shown relative to it.
fn describe_call(name: &str, input: &serde_json::Value, workspace: Option<&Path>) -> String {
    let field = |key: &str| input.get(key).and_then(serde_json::Value::as_str);
    let relative = |path: &str| {
        let shown = workspace
            .and_then(|root| Path::new(path).strip_prefix(root).ok())
            .map_or_else(|| path.to_owned(), |p| p.to_string_lossy().into_owned());
        shown.replace('\\', "/")
    };
    match name {
        "Bash" | "PowerShell" => format!("$ {}", clip_line(field("command").unwrap_or(""), 120)),
        _ => match field("file_path").or_else(|| field("path")).or_else(|| field("notebook_path")) {
            Some(path) => format!("{name} {}", relative(path)),
            None => match field("pattern").or_else(|| field("description")).or_else(|| field("query")) {
                Some(what) => format!("{name} {}", clip_line(what, 100)),
                None => name.to_owned(),
            },
        },
    }
}

/// One line of at most `room` characters, with an ellipsis when it gives some up.
fn clip_line(text: &str, room: usize) -> String {
    let line = text.lines().next().unwrap_or("");
    if line.chars().count() <= room {
        line.to_owned()
    } else {
        format!("{}…", line.chars().take(room.saturating_sub(1)).collect::<String>())
    }
}

impl ModelProvider for ClaudeCode {
    fn complete(&self, request: CompletionRequest) -> CompletionFuture<'_> {
        Box::pin(self.run(request))
    }

    fn has_own_tools(&self) -> bool {
        true
    }
}

/// The system messages of a request, joined into the one system prompt the command line takes.
fn system_prompt(messages: &[Message]) -> String {
    messages.iter().filter(|m| m.role == "system").map(|m| m.content.as_str()).collect::<Vec<_>>().join("\n\n")
}

/// The prompt of a request: a lone user message as it is, a longer conversation as a transcript
/// with the roles named, so the harness sees who said what.
fn prompt_of(messages: &[Message]) -> String {
    let turns: Vec<&Message> = messages.iter().filter(|m| m.role != "system").collect();
    match turns.as_slice() {
        [one] if one.role == "user" => one.content.clone(),
        _ => turns.iter().map(|m| format!("{}: {}", m.role, m.content)).collect::<Vec<_>>().join("\n\n"),
    }
}

/// The result object of `--output-format json`.
#[derive(Deserialize)]
struct WireResult {
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    is_error: bool,
    #[serde(default)]
    subtype: Option<String>,
    #[serde(default)]
    api_error_status: Option<u16>,
    #[serde(default)]
    usage: Option<WireUsage>,
    #[serde(default)]
    total_cost_usd: Option<f64>,
    #[serde(default)]
    num_turns: u64,
    #[serde(default)]
    permission_denials: Vec<serde_json::Value>,
    #[serde(default)]
    session_id: Option<String>,
}

#[derive(Deserialize)]
struct WireUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
}

/// Reads the result JSON into a completion, or the error it reports.
fn parse_result(stdout: &str) -> Result<Completion, ProviderError> {
    let parsed: WireResult = serde_json::from_str(stdout.trim())
        .map_err(|e| ProviderError::Other(format!("unparsable claude result: {e}; output starts with: {}", snippet(stdout))))?;
    let text = parsed.result.unwrap_or_default();
    if parsed.is_error {
        let what = format!("{} ({})", text.trim(), parsed.subtype.as_deref().unwrap_or("error"));
        return Err(match parsed.api_error_status {
            // a missing login comes with no status at all
            _ if text.contains("Not logged in") => ProviderError::Auth(what),
            Some(401 | 403) => ProviderError::Auth(what),
            Some(404) => ProviderError::UnknownModel(what),
            Some(429 | 500..=599) => ProviderError::Unavailable(what),
            _ => ProviderError::Other(what),
        });
    }
    if !parsed.permission_denials.is_empty() {
        tracing::warn!("claude: {} tool call(s) were denied in {} turn(s)", parsed.permission_denials.len(), parsed.num_turns);
    }
    let usage = parsed.usage.map_or_else(Usage::default, |u| Usage {
        // the prompt is everything read, cached or not; the cache split stays visible in cached_tokens
        prompt_tokens: u.input_tokens + u.cache_read_input_tokens + u.cache_creation_input_tokens,
        completion_tokens: u.output_tokens,
        cached_tokens: u.cache_read_input_tokens,
        cost_usd: parsed.total_cost_usd,
    });
    let session_id = parsed.session_id.filter(|s| !s.trim().is_empty());
    Ok(Completion { text, usage, tool_calls: Vec::new(), session_id, gotchas: Vec::new() })
}

/// The gotchas one stream event shows: a command about to run that the catalogue would refuse
/// or rewrite, a shell result with a known error in it, the final answer with a claim in it, and
/// a command the harness refused that no gotcha names (`unmatched-denied`, for the catalogue
/// to grow). `tools` remembers which tool each call id belongs to, across events, so only the
/// output of a shell is scanned: a file the worker reads may quote any error there is.
fn gotchas_of_event(
    catalogue: &Catalogue,
    line: &str,
    conditions: Conditions,
    tools: &mut std::collections::HashMap<String, String>,
) -> Vec<Hit> {
    use serde_json::Value;
    let Ok(event) = serde_json::from_str::<Value>(line) else { return Vec::new() };
    let text_of = |value: &Value| -> String {
        match value {
            Value::String(s) => s.clone(),
            Value::Array(blocks) => blocks.iter().filter_map(|b| b.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"),
            _ => String::new(),
        }
    };
    let mut hits = Vec::new();
    match event.get("type").and_then(Value::as_str) {
        Some("assistant") => {
            for block in event.pointer("/message/content").and_then(Value::as_array).into_iter().flatten() {
                if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                    continue;
                }
                if let (Some(id), Some(name)) = (block.get("id").and_then(Value::as_str), block.get("name").and_then(Value::as_str)) {
                    tools.insert(id.to_owned(), name.to_owned());
                }
                if !matches!(block.get("name").and_then(Value::as_str), Some("Bash" | "PowerShell")) {
                    continue;
                }
                let Some(command) = block.pointer("/input/command").and_then(Value::as_str) else { continue };
                match catalogue.command(command, conditions) {
                    Verdict::Pass => {}
                    Verdict::Deny { id, reason } | Verdict::Rewrite { id, reason, .. } => {
                        hits.push(Hit { id, message: reason, excerpt: snippet(command.lines().next().unwrap_or("")) });
                    }
                }
            }
        }
        Some("user") => {
            for block in event.pointer("/message/content").and_then(Value::as_array).into_iter().flatten() {
                if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                    continue;
                }
                // only what a shell said; a file the worker read is not the machine talking
                let shell = block
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .and_then(|id| tools.get(id))
                    .is_some_and(|name| matches!(name.as_str(), "Bash" | "PowerShell"));
                if shell {
                    hits.extend(catalogue.scan(Where::Result, &text_of(block.get("content").unwrap_or(&Value::Null))));
                }
            }
        }
        Some("result") => {
            hits.extend(catalogue.scan(Where::Answer, event.get("result").and_then(Value::as_str).unwrap_or("")));
            for denial in event.get("permission_denials").and_then(Value::as_array).into_iter().flatten() {
                let command = denial.pointer("/tool_input/command").and_then(Value::as_str).unwrap_or("");
                // a PowerShell denial is the wrapper, whatever the command was
                if denial.get("tool_name").and_then(Value::as_str) == Some("PowerShell") {
                    let message = catalogue.get("powershell-wrapped").map(|g| g.message.clone()).unwrap_or_default();
                    hits.push(Hit { id: "powershell-wrapped".to_owned(), message, excerpt: snippet(command.lines().next().unwrap_or("")) });
                    continue;
                }
                if !command.is_empty() && catalogue.command(command, conditions) == Verdict::Pass {
                    hits.push(Hit {
                        id: ra_gotchas::UNMATCHED.to_owned(),
                        message: "the harness refused a command no gotcha names; see rc gotchas".to_owned(),
                        excerpt: snippet(command.lines().next().unwrap_or("")),
                    });
                }
            }
        }
        _ => {}
    }
    hits
}

/// The `--settings` JSON that routes the harness's tool hooks to `rc hook pre` and `rc hook
/// post`, so a command the catalogue knows is refused or rewritten before it runs and a result
/// it knows gets its hint right after, with no model call.
fn hook_settings(rc: &Path) -> String {
    // the JSON encoder escapes the backslashes of a Windows path itself
    let command = |event: &str| format!("\"{}\" hook {event}", rc.display());
    serde_json::json!({
        "hooks": {
            "PreToolUse": [{"matcher": "Bash|PowerShell", "hooks": [{"type": "command", "command": command("pre"), "timeout": 10}]}],
            "PostToolUse": [{"matcher": "Bash|PowerShell", "hooks": [{"type": "command", "command": command("post"), "timeout": 10}]}]
        }
    })
    .to_string()
}

/// The first 200 characters of a text, for an error message.
fn snippet(text: &str) -> String {
    let mut out: String = text.chars().take(200).collect();
    if text.chars().count() > 200 {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests;
