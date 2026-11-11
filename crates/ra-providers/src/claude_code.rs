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

use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

use crate::{Completion, CompletionFuture, CompletionRequest, Message, ModelProvider, ProviderError, Usage};

/// The Claude Code provider.
pub struct ClaudeCode {
    /// The command; `claude` on the path unless told otherwise.
    binary: PathBuf,
    /// The most turns a worker run may take before the harness stops it.
    max_turns: u32,
    /// The shell commands a worker may run, as Claude Code permission rules such as `Bash(cargo *)`.
    allowed_shell: Vec<String>,
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
            allowed_shell: ["Bash(cargo *)", "Bash(git status*)", "Bash(git diff*)", "Bash(git log*)"].map(str::to_owned).to_vec(),
        }
    }

    /// Runs another binary, for a pinned install or a test double.
    pub fn with_binary(mut self, binary: impl Into<PathBuf>) -> Self {
        self.binary = binary.into();
        self
    }

    /// The most turns a worker run may take.
    pub fn with_max_turns(mut self, turns: u32) -> Self {
        self.max_turns = turns;
        self
    }

    /// The command-line arguments of one request. The prompt goes on stdin, so its length is no
    /// concern of the argument list.
    fn args(&self, request: &CompletionRequest) -> Vec<String> {
        // not --bare: it skips the hooks and plugins, but it also skips the OAuth login the
        // subscription route runs on (card #11 found out)
        // stream-json: the events arrive as the harness works, so the console can show the work
        let mut args = ["-p", "--output-format", "stream-json", "--verbose", "--no-session-persistence"].map(str::to_owned).to_vec();
        args.extend(["--model".to_owned(), request.model.clone()]);
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
        if let Some(pipe) = child.stdout.take() {
            let mut lines = tokio::io::BufReader::new(pipe).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                for step in progress_of(&line, request.workspace.as_deref()) {
                    tracing::info!(target: "ra::progress", "{step}");
                }
                if line.contains(r#""type":"result""#) {
                    result = Some(line.clone());
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
            Some(result) => parse_result(&result),
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
    Ok(Completion { text, usage, tool_calls: Vec::new() })
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
