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

use crate::{Completion, CompletionFuture, CompletionRequest, Message, ModelProvider, ProviderError, TurnUsage, Usage};

/// Where the plan's limits are kept, beside the roadmap.
pub const LIMITS_FILE: &str = "var/limits.json";
/// Where every `rate_limit_event` is appended, for the week's history.
pub const LIMITS_LOG: &str = "var/limits.log";
/// Where the event stream of the latest workspace run is kept, one JSON line per event, for
/// the console's `log` to save with the run. A new run replaces it.
pub const STREAM_FILE: &str = "var/runs/last.jsonl";

/// The built-in tools a worker gets: the files, the search, both shells, the web and the plan.
/// Not the subagents: one card is one worker's work, and a subagent only multiplies the context
/// it reads.
const WORKER_TOOLS: &str = "Bash,PowerShell,Read,Edit,Write,Glob,Grep,TodoWrite,WebFetch,WebSearch,NotebookEdit";

/// The settings a worker loads: the project's and the local ones, not the user's. The user's
/// hooks are written for their interactive sessions (on this machine one denies Bash outright
/// and another wraps every PowerShell command in a logging prelude no rule can read), and a
/// worker that inherits them has no shell at all. The gotcha hooks come in by `--settings`.
pub const WORKER_SETTING_SOURCES: &str = "project,local";

/// The Claude Code provider.
pub struct ClaudeCode {
    /// The command; `claude` on the path unless told otherwise.
    binary: PathBuf,
    /// The most turns a worker run may take before the harness stops it, when a request names
    /// no cap of its own; `None`, the default, caps nothing.
    max_turns: Option<u32>,
    /// The shell commands a worker may run, as Claude Code permission rules such as `Bash(cargo *)`.
    allowed_shell: Vec<String>,
    /// The effort the harness asks of the model: `medium` unless told otherwise, the efficient
    /// setting for the routine card; a hard one can ask for more.
    effort: String,
    /// The `rc` binary the harness's tool hooks call, when the gotchas guard a run.
    hooks: Option<PathBuf>,
    /// The gotchas the stream is read against, when there is a catalogue.
    catalogue: Option<Catalogue>,
    /// The environment the harness runs with, on top of this process's own.
    env: Vec<(String, String)>,
    /// The programs of the project's own `[worker] allow` rules, which the allowlist gotcha
    /// does not count against a run.
    own_prefixes: Vec<String>,
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
            max_turns: None,
            // the build, the read-only git commands, and the read-only shell in both shells: a
            // denied `ls` costs a turn and teaches nothing. A pipe is judged by its parts, so
            // `cargo test 2>&1 | tail -20` passes and keeps the output short
            allowed_shell: [
                "Bash(cargo *)",
                "Bash(rustfmt *)",
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
                // a word to the log, and the one more git question that only reads
                "Bash(echo *)",
                "Bash(git check-ignore*)",
                // the text filters a pipe ends in, so a long output can be narrowed
                "Bash(sort*)",
                "Bash(uniq*)",
                "Bash(cut *)",
                "Bash(paste *)",
                "Bash(tr *)",
                "PowerShell(cargo *)",
                "PowerShell(rustfmt *)",
                "PowerShell(git status*)",
                "PowerShell(git diff*)",
                "PowerShell(git log*)",
                "PowerShell(git show*)",
                "PowerShell(Get-ChildItem*)",
                "PowerShell(Get-Content *)",
                "PowerShell(Select-String *)",
                "PowerShell(Select-Object *)",
                "PowerShell(Test-Path *)",
                "PowerShell(Get-Location*)",
                "PowerShell(New-Item *)",
                "PowerShell(Write-Output *)",
                "PowerShell(git check-ignore*)",
            ]
            .map(str::to_owned)
            .to_vec(),
            effort: "medium".to_owned(),
            hooks: None,
            catalogue: None,
            env: Vec::new(),
            own_prefixes: Vec::new(),
        }
    }

    /// More permission rules for the worker, a project's own commands, on top of the defaults.
    #[must_use]
    pub fn with_allowed(mut self, rules: impl IntoIterator<Item = String>) -> Self {
        for rule in rules {
            if let Some(prefix) = rule_prefix(&rule)
                && !self.own_prefixes.contains(&prefix)
            {
                self.own_prefixes.push(prefix);
            }
            if !self.allowed_shell.contains(&rule) {
                self.allowed_shell.push(rule);
            }
        }
        self
    }

    /// Whether a command starts with one of the project's own allowed programs: such a command
    /// is outside the shell allowlist by design, and the allowlist gotcha does not count it.
    fn own_command(&self, command: &str) -> bool {
        let command = command.trim_start();
        self.own_prefixes
            .iter()
            .any(|p| command.starts_with(p.as_str()) && command[p.len()..].chars().next().is_none_or(char::is_whitespace))
    }

    /// Environment the harness runs with, a project's `[worker.env]`.
    #[must_use]
    pub fn with_env(mut self, env: impl IntoIterator<Item = (String, String)>) -> Self {
        self.env.extend(env);
        self
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

    /// The most turns a worker run may take, when a request names none of its own.
    pub fn with_max_turns(mut self, turns: u32) -> Self {
        self.max_turns = Some(turns);
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
        // the card's effort when the request carries one, else the provider's
        let effort = request.effort.as_deref().map(str::trim).filter(|e| !e.is_empty()).unwrap_or(&self.effort);
        args.extend(["--model".to_owned(), request.model.clone(), "--effort".to_owned(), effort.to_owned()]);
        if let Some(session) = request.resume.as_deref().filter(|s| !s.trim().is_empty()) {
            // the harness picks up where the last run of the task left off, its context intact
            args.extend(["--resume".to_owned(), session.trim().to_owned()]);
        }
        let system = system_prompt(&request.messages);
        if !system.is_empty() {
            // a worker's system prompt goes by file: on the command line it ran past Windows'
            // limit (os error 206) once the project context, the other projects' digests and
            // the machine notes were in it; a chat completion's short one stays inline
            match request.workspace.as_deref().and_then(|dir| write_system_prompt(dir, &system).ok()) {
                Some(path) => args.extend(["--system-prompt-file".to_owned(), path.display().to_string()]),
                None => args.extend(["--system-prompt".to_owned(), system]),
            }
        }
        if request.workspace.is_some() {
            // a worker: the file tools and the allowed shell commands, inside the workspace, and
            // no MCP servers: the project's .mcp.json would hand it the agent itself, holding
            // target/debug/rc.exe for the run and a `run` tool of its own
            args.extend(["--permission-mode".to_owned(), "acceptEdits".to_owned(), "--strict-mcp-config".to_owned()]);
            // a cap only when the request or the provider names one; none stands by default
            if let Some(cap) = request.max_turns.or(self.max_turns) {
                args.extend(["--max-turns".to_owned(), cap.to_string()]);
            }
            if !self.allowed_shell.is_empty() {
                args.push("--allowedTools".to_owned());
                args.extend(self.allowed_shell.iter().cloned());
            }
            // the built-in tools of a worker, and the project's settings only: with the user's
            // hooks along, the worker on this machine had no shell (card #25 found out)
            args.extend(["--tools".to_owned(), WORKER_TOOLS.to_owned(), "--setting-sources".to_owned(), WORKER_SETTING_SOURCES.to_owned()]);
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
        command.envs(self.env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
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
        // the files the worker read, in order, for the run line to name the ones it read again
        let mut reads: Vec<String> = Vec::new();
        // the tokens of each assistant turn, as the events show them
        let mut turns = TurnTally::default();
        // the project's context cap, and the size of the turn that crossed it once one did
        let mut watch = ContextWatch::new(request.max_context);
        let mut crossed: Option<u64> = None;
        // which tool each call id belongs to, so only a shell's output is read for gotchas
        let mut tools: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        // a console holds the binary while var/console.pid is in the workspace
        let conditions = Conditions { console_running: request.workspace.as_deref().is_some_and(|w| w.join("var/console.pid").exists()) };
        let mut stream = Stream::open(request.workspace.as_deref());
        if let Some(pipe) = child.stdout.take() {
            let mut lines = tokio::io::BufReader::new(pipe).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                stream.note(&line);
                for step in progress_of(&line, request.workspace.as_deref()) {
                    tracing::info!(target: "ra::progress", "{step}");
                }
                turns.note(&line);
                if let Some(size) = watch.note(&line, &turns) {
                    // the turns past this one would each read the whole context again: the
                    // child stops here and the caller continues the task in a fresh session
                    tracing::info!(target: "ra::progress", "⏹ the context cap: a turn of {size} tokens");
                    crossed = Some(size);
                    let _ = child.start_kill();
                    break;
                }
                reads.extend(reads_of(&line, request.workspace.as_deref()));
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
                        // the project's own commands are past the allowlist by design: the
                        // hook lets them run, and the count leaves them alone too
                        if hit.id == "shell-outside-allowlist" && self.own_command(&hit.excerpt) {
                            continue;
                        }
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
                    let root = request.workspace.clone().unwrap_or_default();
                    if let Err(e) = newer.append(&root.join(LIMITS_LOG)) {
                        tracing::warn!("cannot write the limits log: {e:#}");
                    }
                    let path = root.join(LIMITS_FILE);
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
        // a result names the reason itself, exit code or not
        if let Some(result) = result {
            with_stream(parse_result(&result), |completion| {
                completion.gotchas = gotchas;
                completion.turn_usage = turns.into_turns();
                completion.reads = reads;
            })
        } else if let Some((size, cap)) = crossed.zip(request.max_context) {
            let mut completion = turns.completion();
            completion.gotchas = gotchas;
            completion.reads = reads;
            Err(ProviderError::ContextOut { completion: Box::new(completion), cap, size })
        } else {
            let reason = stderr.lines().map(str::trim).filter(|l| !l.is_empty()).next_back().unwrap_or(last_line.trim());
            let reason = if reason.is_empty() { "no output" } else { reason };
            Err(ProviderError::Other(format!("claude exited with {status} and no result: {}", snippet(reason))))
        }
    }
}

/// The event stream of a run, written to [`STREAM_FILE`] as it comes, so a run that ends badly
/// still leaves what the harness said. A chat completion, without a workspace, leaves none.
struct Stream(Option<std::fs::File>);

impl Stream {
    /// Opens the stream file under `workspace`, in place of the one before.
    fn open(workspace: Option<&Path>) -> Self {
        let file = workspace.and_then(|root| {
            let path = root.join(STREAM_FILE);
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            match std::fs::File::create(&path) {
                Ok(file) => Some(file),
                Err(e) => {
                    tracing::warn!("cannot keep the run's stream at {}: {e}", path.display());
                    None
                }
            }
        });
        Self(file)
    }

    /// Appends one event line.
    fn note(&mut self, line: &str) {
        if let Some(file) = &mut self.0
            && !line.trim().is_empty()
        {
            let _ = std::io::Write::write_all(file, format!("{line}\n").as_bytes());
        }
    }
}

/// The tokens of the assistant turns of a stream, one record per turn. The harness emits one
/// `assistant` event per content block, all with the message id and the usage of the turn, so
/// the events of one id fold into one record and the last one read stands.
#[derive(Default)]
struct TurnTally {
    turns: Vec<(Option<String>, TurnUsage)>,
    /// The cached share of each turn's input, for the usage of a run that ends without a result.
    cached: Vec<u64>,
    /// The text blocks of the last turn, the worker's words when the run ends without a result.
    said: Vec<String>,
}

impl TurnTally {
    /// Books the turn one stream event belongs to; any other event is passed over.
    fn note(&mut self, line: &str) {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else { return };
        if event.get("type").and_then(serde_json::Value::as_str) != Some("assistant") {
            return;
        }
        let Some(usage) = event.pointer("/message/usage") else { return };
        let count = |key: &str| usage.get(key).and_then(serde_json::Value::as_u64).unwrap_or(0);
        let turn = TurnUsage {
            // the input is everything read, cached or not, as the result's usage counts it
            input_tokens: count("input_tokens") + count("cache_read_input_tokens") + count("cache_creation_input_tokens"),
            output_tokens: count("output_tokens"),
        };
        let id = event.pointer("/message/id").and_then(serde_json::Value::as_str).map(str::to_owned);
        let text = event
            .pointer("/message/content")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|block| block.get("text").and_then(serde_json::Value::as_str))
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty());
        match self.turns.last_mut() {
            Some((last, booked)) if id.is_some() && *last == id => {
                *booked = turn;
                if let Some(cached) = self.cached.last_mut() {
                    *cached = count("cache_read_input_tokens");
                }
                self.said.extend(text);
            }
            _ => {
                self.turns.push((id, turn));
                self.cached.push(count("cache_read_input_tokens"));
                self.said = text.collect();
            }
        }
    }

    /// The input of the last turn, when the stream showed one.
    fn last_input(&self) -> Option<u64> {
        self.turns.last().map(|(_, turn)| turn.input_tokens)
    }

    /// The completion of a run stopped before its result: the usage summed over the turns the
    /// stream showed, at no reported cost, and the last turn's words.
    fn completion(self) -> Completion {
        let turns = u64::try_from(self.turns.len()).unwrap_or(u64::MAX);
        let usage = Usage {
            prompt_tokens: self.turns.iter().map(|(_, turn)| turn.input_tokens).sum(),
            completion_tokens: self.turns.iter().map(|(_, turn)| turn.output_tokens).sum(),
            cached_tokens: self.cached.iter().sum(),
            cost_usd: None,
            cost_computed: false,
        };
        Completion {
            text: self.said.join("\n"),
            usage,
            turns,
            turn_usage: self.into_turns(),
            tool_calls: Vec::new(),
            session_id: None,
            gotchas: Vec::new(),
            reads: Vec::new(),
        }
    }

    fn into_turns(self) -> Vec<TurnUsage> {
        self.turns.into_iter().map(|(_, turn)| turn).collect()
    }
}

/// The turns of a run the context cap leaves alone: a fresh session spends them reading its
/// files in, and a resumed one answers its verdict in them. A cap that fell in them ended a
/// run on what the next one would read again (runelite #15 at turn 7, rhipper-asm #134's
/// resumed rerun at turn 1).
pub const READING_IN_TURNS: usize = 12;

/// Watches a stream for the turn whose input passes the context cap, past the reading-in
/// turns. The run stops after that turn, once its tool results are in, so an edit in flight
/// lands and the handoff can name it; a result that comes first means the run finished on its
/// own and stands.
struct ContextWatch {
    cap: Option<u64>,
    /// The input of the turn that crossed the cap, once one has.
    crossed: Option<u64>,
}

impl ContextWatch {
    fn new(cap: Option<u64>) -> Self {
        Self { cap, crossed: None }
    }

    /// Reads one event after the tally has: the size of the crossing turn once the run may
    /// stop, `None` while it goes on.
    fn note(&mut self, line: &str, turns: &TurnTally) -> Option<u64> {
        let cap = self.cap?;
        if line.contains(r#""type":"assistant""#) {
            if turns.turns.len() > READING_IN_TURNS
                && let Some(input) = turns.last_input().filter(|input| *input > cap)
            {
                self.crossed = Some(input);
            }
            return None;
        }
        self.crossed.filter(|_| line.contains(r#""type":"user""#))
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

/// The files one stream event reads: the `file_path` of each `Read` call in an assistant event,
/// relative to the workspace. Nothing from any other event.
fn reads_of(line: &str, workspace: Option<&Path>) -> Vec<String> {
    use serde_json::Value;
    if !line.contains(r#""name":"Read""#) {
        return Vec::new();
    }
    let Ok(event) = serde_json::from_str::<Value>(line) else { return Vec::new() };
    if event.get("type").and_then(Value::as_str) != Some("assistant") {
        return Vec::new();
    }
    event
        .pointer("/message/content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| {
            block.get("type").and_then(Value::as_str) == Some("tool_use") && block.get("name").and_then(Value::as_str) == Some("Read")
        })
        .filter_map(|block| block.pointer("/input/file_path").and_then(Value::as_str))
        .map(|path| relative(path, workspace))
        .collect()
}

/// A path inside the workspace relative to it, with forward slashes; another path as it came.
fn relative(path: &str, workspace: Option<&Path>) -> String {
    let shown = workspace
        .and_then(|root| Path::new(path).strip_prefix(root).ok())
        .map_or_else(|| path.to_owned(), |p| p.to_string_lossy().into_owned());
    shown.replace('\\', "/")
}

/// One tool call as a line: `$ cargo test` for a command, `Read src/main.rs` for a file tool,
/// `Grep pattern` for a search; a path inside the workspace is shown relative to it.
fn describe_call(name: &str, input: &serde_json::Value, workspace: Option<&Path>) -> String {
    let field = |key: &str| input.get(key).and_then(serde_json::Value::as_str);
    match name {
        "Bash" | "PowerShell" => format!("$ {}", clip_line(field("command").unwrap_or(""), 120)),
        _ => match field("file_path").or_else(|| field("path")).or_else(|| field("notebook_path")) {
            Some(path) => format!("{name} {}", relative(path, workspace)),
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

    fn command_line(&self, request: &CompletionRequest) -> Option<Vec<String>> {
        let mut line = vec![self.binary.display().to_string()];
        line.extend(self.args(request));
        Some(line)
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

/// Adds what the event stream showed to the parsed result: to the completion, and to the one a
/// run out of turns carries, which is booked like a finished run's; any other error has none.
fn with_stream(parsed: Result<Completion, ProviderError>, seen: impl FnOnce(&mut Completion)) -> Result<Completion, ProviderError> {
    match parsed {
        Ok(mut completion) => {
            seen(&mut completion);
            Ok(completion)
        }
        Err(ProviderError::TurnsOut(mut completion)) => {
            seen(&mut completion);
            Err(ProviderError::TurnsOut(completion))
        }
        Err(error) => Err(error),
    }
}

/// Reads the result JSON into a completion, or the error it reports.
fn parse_result(stdout: &str) -> Result<Completion, ProviderError> {
    let parsed: WireResult = serde_json::from_str(stdout.trim())
        .map_err(|e| ProviderError::Other(format!("unparsable claude result: {e}; output starts with: {}", snippet(stdout))))?;
    let text = parsed.result.unwrap_or_default();
    // the turns running out is an error to the harness, but the result still carries what the
    // run read, wrote and cost: that goes back with the error, for the ledger
    let turns_out = parsed.is_error && parsed.subtype.as_deref() == Some("error_max_turns");
    if parsed.is_error && !turns_out {
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
        cost_computed: false,
    });
    let session_id = parsed.session_id.filter(|s| !s.trim().is_empty());
    let completion = Completion {
        text,
        usage,
        turns: parsed.num_turns,
        turn_usage: Vec::new(),
        tool_calls: Vec::new(),
        session_id,
        gotchas: Vec::new(),
        reads: Vec::new(),
    };
    if turns_out { Err(ProviderError::TurnsOut(Box::new(completion))) } else { Ok(completion) }
}

/// Writes a run's system prompt under the workspace's `var/runs`, one file per process, so two
/// runs in one project do not overwrite each other's; returns the path.
fn write_system_prompt(workspace: &Path, text: &str) -> std::io::Result<PathBuf> {
    let dir = workspace.join("var").join("runs");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("system-{}.md", std::process::id()));
    std::fs::write(&path, text)?;
    Ok(path)
}

/// The program a permission rule names: `python` of `Bash(python *)`, `gw.ps1` of
/// `PowerShell(.\gw.ps1 *)` is `.\gw.ps1`; none for a rule with nothing inside.
fn rule_prefix(rule: &str) -> Option<String> {
    let inner = match rule.find('(') {
        Some(at) => rule[at + 1..].trim_end_matches(')'),
        None => rule,
    };
    let prefix = inner.trim().trim_end_matches('*').trim_end();
    (!prefix.is_empty()).then(|| prefix.to_owned())
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
                // the worker's own words: an edit it says went wrong is a turn lost
                if block.get("type").and_then(Value::as_str) == Some("text") {
                    hits.extend(catalogue.scan(Where::Text, block.get("text").and_then(Value::as_str).unwrap_or("")));
                    continue;
                }
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
                // what a shell said, and what an edit tool said; a file the worker read is not
                // the machine talking
                let tool = block.get("tool_use_id").and_then(Value::as_str).and_then(|id| tools.get(id)).map(String::as_str);
                let place = match tool {
                    Some("Bash" | "PowerShell") => Where::Result,
                    Some("Edit" | "Write" | "MultiEdit" | "NotebookEdit") => Where::Edit,
                    _ => continue,
                };
                hits.extend(catalogue.scan(place, &text_of(block.get("content").unwrap_or(&Value::Null))));
            }
        }
        Some("result") => {
            hits.extend(catalogue.scan(Where::Answer, event.get("result").and_then(Value::as_str).unwrap_or("")));
            for denial in event.get("permission_denials").and_then(Value::as_array).into_iter().flatten() {
                let command = denial.pointer("/tool_input/command").and_then(Value::as_str).unwrap_or("");
                // a rewrite is no reason to refuse: only a gotcha that would deny it names one
                if !command.is_empty() && !matches!(catalogue.command(command, conditions), Verdict::Deny { .. }) {
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
/// post`, so a command the catalogue knows is refused or rewritten before it runs, a result
/// it knows gets its hint right after, and a whole-file Read is answered with the outline,
/// with no model call.
fn hook_settings(rc: &Path) -> String {
    // the JSON encoder escapes the backslashes of a Windows path itself
    let command = |event: &str| format!("\"{}\" hook {event}", rc.display());
    serde_json::json!({
        "hooks": {
            "PreToolUse": [{"matcher": "Bash|PowerShell", "hooks": [{"type": "command", "command": command("pre"), "timeout": 10}]}],
            "PostToolUse": [{"matcher": "Bash|PowerShell|Edit|Write|MultiEdit|NotebookEdit|Read", "hooks": [{"type": "command", "command": command("post"), "timeout": 10}]}]
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
