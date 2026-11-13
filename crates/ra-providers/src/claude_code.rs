//! The Claude Code adapter: one headless `claude -p` run per completion, on the machine's own
//! Claude login, so a subscription pays for the work and no API key is involved.
//!
//! Claude Code is a harness, not a bare model: it brings its own tools. A request with a
//! workspace lets it act there with its file tools and a short list of shell commands; a request
//! without one runs with no tools and one turn, a plain chat completion. The `tools` of the
//! request are not forwarded: the harness owns its tool set, and the answer carries no tool
//! calls. `temperature` and `max_tokens` have no flag on the command line and are not sent.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use ra_gotchas::{Catalogue, Conditions, Hit, Journal, Verdict, Where};
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

use crate::openrouter::Tokens;
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

/// The built-in tools of a read-only run, an investigator: it reads and searches the tree and
/// the web, and has no shell to build with and no tool to edit with.
pub const READER_TOOLS: &str = "Read,Glob,Grep,WebFetch,WebSearch";

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
    /// The directories of the machine's tools, from the hub's manifest, which the harness's
    /// PATH gets after its own: a tool the probe found off the PATH is found by the worker too.
    tool_dirs: Vec<PathBuf>,
    /// The programs of the project's own `[worker] allow` rules, which the allowlist gotcha
    /// does not count against a run.
    own_prefixes: Vec<String>,
    /// The project's check is the full suite, so the count reads a worker's own run of it as
    /// the hook narrows it: to the crates the run has changed.
    suite_check: bool,
    /// The directories outside the workspace a run may read: the project's declared read paths,
    /// the toolchain caches its builds use, what it carries into a worktree.
    read_roots: Vec<PathBuf>,
}

/// The run's own temp directory under its workspace: a test's files land where the run may read
/// them, not under a system temp outside the worktree.
pub const RUN_TEMP: &str = "var/tmp";

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
            // denied `ls` costs a turn and teaches nothing. The table is the hook's too, which
            // judges a chain or a pipe by its parts and answers for the shapes this list cannot
            allowed_shell: ra_gotchas::shape::rules(),
            effort: "medium".to_owned(),
            hooks: None,
            catalogue: None,
            env: Vec::new(),
            tool_dirs: Vec::new(),
            own_prefixes: Vec::new(),
            suite_check: false,
            read_roots: Vec::new(),
        }
    }

    /// Directories outside the workspace a run may read, passed to the harness as added
    /// directories; one named twice goes once.
    #[must_use]
    pub fn with_read_roots(mut self, roots: impl IntoIterator<Item = PathBuf>) -> Self {
        for root in roots {
            if !self.read_roots.contains(&root) {
                self.read_roots.push(root);
            }
        }
        self
    }

    /// The environment of a run on top of this process's own: `run_env` of its workspace and
    /// the run's temp directory under it. A key the project names itself keeps the project's
    /// value.
    #[must_use]
    pub fn environment(&self, request: &CompletionRequest) -> Vec<(String, String)> {
        let env = self.run_env(request.workspace.as_deref()).into_iter().map(|(k, v)| (k, v.to_string_lossy().into_owned()));
        env.chain(self.temp_env(request)).collect()
    }

    /// The run's temp directory as `TMP`, `TEMP` and `TMPDIR`, each one the project does not set
    /// itself: what a test writes to temp lands inside the worktree, where the run may read it
    /// back.
    fn temp_env(&self, request: &CompletionRequest) -> Vec<(String, String)> {
        let Some(temp) = run_temp(request) else { return Vec::new() };
        ["TMP", "TEMP", "TMPDIR"]
            .into_iter()
            .filter(|key| !self.env.iter().any(|(k, _)| k.eq_ignore_ascii_case(key)))
            .map(|key| (key.to_owned(), temp.display().to_string()))
            .collect()
    }

    /// Whether the project's check is the full suite, the orchestrator's after the report.
    #[must_use]
    pub fn with_suite_check(mut self, suite: bool) -> Self {
        self.suite_check = suite;
        self
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

    /// The directories of the machine's tools, the hub's manifest's, for the harness's PATH.
    #[must_use]
    pub fn with_tool_dirs(mut self, dirs: impl IntoIterator<Item = PathBuf>) -> Self {
        for dir in dirs {
            if !self.tool_dirs.contains(&dir) {
                self.tool_dirs.push(dir);
            }
        }
        self
    }

    /// The environment a run in `workspace` sets on top of this process's: the project's
    /// variables; the PATH with this binary's directory first and the tools' directories after
    /// it; the worktree's cargo target; and on Windows `MSYS_NO_PATHCONV`, so Git Bash hands a
    /// `/c` or a `C:/…` argument to the program as written. A project's own setting wins.
    #[must_use]
    pub fn run_env(&self, workspace: Option<&Path>) -> Vec<(String, std::ffi::OsString)> {
        let mut env: Vec<(String, std::ffi::OsString)> = self.env.iter().map(|(k, v)| (k.clone(), v.into())).collect();
        let set = |env: &[(String, std::ffi::OsString)], key: &str| env.iter().any(|(k, _)| k.eq_ignore_ascii_case(key));
        // a run in a worktree builds in the project's target, like the run beside it: one
        // build directory under one lock, not one per worktree
        if !set(&env, "CARGO_TARGET_DIR")
            && let Some(target) = workspace.and_then(shared_target)
        {
            env.push(("CARGO_TARGET_DIR".to_owned(), target.into_os_string()));
        }
        // the worker's shell finds `rc` (its roadmap commands, `rc hold`) beside the binary that
        // runs it: the PATH a walk inherits can predate the install directory's place on it, and
        // every `rc show` of the agent's regeneration card was `command not found` (2026-11-12)
        if !set(&env, "PATH") {
            let own = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf));
            let path = std::env::var_os("PATH").unwrap_or_default();
            if let Some(path) = path_with(own, &self.tool_dirs, &path) {
                env.push(("PATH".to_owned(), path));
            }
        }
        if cfg!(windows) && !set(&env, "MSYS_NO_PATHCONV") {
            env.push(("MSYS_NO_PATHCONV".to_owned(), "1".into()));
        }
        // a build the hook does not rewrite (a script, a gradle wrapper) still meets the shell's
        // 120 s default: raise the default and the maximum to ten minutes
        for key in ["BASH_DEFAULT_TIMEOUT_MS", "BASH_MAX_TIMEOUT_MS"] {
            if !set(&env, key) {
                env.push((key.to_owned(), "600000".into()));
            }
        }
        env
    }

    /// The effort level: `low`, `medium`, `high`, `xhigh` or `max`.
    #[must_use]
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

    /// The system prompt of one request: the request's own, and for a worker with a shell the
    /// allowed shapes block, so the rules are read before a refusal has to teach them.
    fn system_of(&self, request: &CompletionRequest) -> String {
        let mut system = system_prompt(&request.messages);
        if request.workspace.is_some() && !request.read_only {
            if !system.is_empty() {
                system.push_str("\n\n");
            }
            system.push_str(&ra_gotchas::shape::block());
            if !self.own_prefixes.is_empty() {
                let _ = writeln!(system, "\n- the project's own commands run too: {}", self.own_prefixes.join(", "));
            }
        }
        system
    }

    /// The command-line arguments of one request. The prompt goes on stdin, so its length is no
    /// concern of the argument list.
    fn args(&self, request: &CompletionRequest) -> Vec<String> {
        // not --bare: it skips the hooks and plugins, but it also skips the OAuth login the
        // subscription route runs on (card #11 found out)
        // stream-json: the events arrive as the harness works, so the console can show the work
        let mut args = ["-p", "--output-format", "stream-json", "--verbose"].map(str::to_owned).to_vec();
        if request.workspace.is_none() || request.read_only {
            // a chat completion leaves no session behind, nor does a read-only run; a worker
            // keeps its own, for the next run of the task to resume
            args.push("--no-session-persistence".to_owned());
        }
        // the card's effort when the request carries one, else the provider's
        let effort = request.effort.as_deref().map(str::trim).filter(|e| !e.is_empty()).unwrap_or(&self.effort);
        args.extend(["--model".to_owned(), request.model.clone(), "--effort".to_owned(), effort.to_owned()]);
        if let Some(session) = request.resume.as_deref().filter(|s| !s.trim().is_empty()) {
            // the harness picks up where the last run of the task left off, its context intact
            args.extend(["--resume".to_owned(), session.trim().to_owned()]);
        }
        let system = self.system_of(request);
        if !system.is_empty() {
            // a worker's system prompt goes by file: on the command line it ran past Windows'
            // limit (os error 206) once the project context, the other projects' digests and
            // the machine notes were in it; a chat completion's short one stays inline
            match request.workspace.as_deref().and_then(|dir| write_system_prompt(dir, &system).ok()) {
                Some(path) => args.extend(["--system-prompt-file".to_owned(), path.display().to_string()]),
                None => args.extend(["--system-prompt".to_owned(), system]),
            }
        }
        if request.read_only && request.workspace.is_some() {
            // an investigator: the reading tools alone, which ask no permission, inside the
            // workspace; no shell, no edits, no session
            args.push("--strict-mcp-config".to_owned());
            if let Some(cap) = request.max_turns.or(self.max_turns) {
                args.extend(["--max-turns".to_owned(), cap.to_string()]);
            }
            args.extend(["--tools".to_owned(), READER_TOOLS.to_owned(), "--setting-sources".to_owned(), WORKER_SETTING_SOURCES.to_owned()]);
        } else if request.workspace.is_some() {
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
        if request.workspace.is_some() {
            // the reads a run needs outside its worktree (a gitignored source tree, the cargo
            // registry, a JDK's src.zip) and its own temp: refused, they cost a turn each, or a
            // worker wrote from memory what it could have read
            let dirs: Vec<String> = self.read_roots.iter().cloned().chain(run_temp(request)).map(|d| d.display().to_string()).collect();
            if !dirs.is_empty() {
                args.push("--add-dir".to_owned());
                args.extend(dirs);
            }
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
        // the scratch drive's Gradle cache, when the machine has one; its temp directory gives
        // way to the run's own below, which the run may read back
        if let Some(dir) = &request.workspace {
            command.envs(scratch_env(dir));
        }
        if let Some(temp) = run_temp(&request) {
            let _ = std::fs::create_dir_all(temp);
        }
        command.envs(self.run_env(request.workspace.as_deref()));
        command.envs(self.temp_env(&request));
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
        // the project's context cap, and the size of the turn that crossed it once one did;
        // a Fable run has none whatever the project sets: its window is the point of it
        let fable = request.model.to_ascii_lowercase().contains("fable");
        let resumed = request.resume.as_deref().is_some_and(|s| !s.trim().is_empty());
        let mut watch = ContextWatch::new(request.max_context.filter(|_| !fable)).resumed(resumed);
        let mut crossed: Option<u64> = None;
        // which tool each call id belongs to, so only a shell's output is read for gotchas
        let mut tools: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        // the workspace binaries a running process holds, asked once as the run starts
        let held = request.workspace.as_deref().map(ra_gotchas::held_packages).unwrap_or_default();
        let mut stream = Stream::open(request.workspace.as_deref());
        if let Some(pipe) = child.stdout.take() {
            let mut lines = tokio::io::BufReader::new(pipe).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                stream.note(&line);
                for step in progress_of(&line, request.workspace.as_deref()) {
                    tracing::info!(target: "ra::progress", "{step}");
                }
                if let Some(turn) = turns.turn_progress(&line) {
                    tracing::info!(target: "ra::progress", "{turn}");
                }
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
                    // the changed crates cost a git status: asked only of an event that runs cargo test
                    let changed = match &request.workspace {
                        Some(workspace) if self.suite_check && line.contains("cargo test") => ra_gotchas::changed_crates(workspace),
                        _ => Vec::new(),
                    };
                    let conditions =
                        Conditions { held_packages: &held, suite_check: self.suite_check, changed_crates: &changed, allowed: false };
                    for hit in gotchas_of_event(catalogue, &line, conditions, &mut tools) {
                        // the project's own commands are past the allowlist by design: the
                        // hook lets them run, and the count leaves them alone too
                        if hit.id == "shell-outside-allowlist" && self.own_command(&hit.excerpt) {
                            continue;
                        }
                        // a build put on a slot is the routing every build takes, not a warning:
                        // a quiet step, out of the journal and the run's count
                        if catalogue.is_routine(&hit.id) {
                            tracing::info!(target: "ra::progress", "↳ on a slot: {}", hit.excerpt);
                            continue;
                        }
                        tracing::info!(target: "ra::progress", "⚠ {}: {}", hit.id, hit.excerpt);
                        // the journal is written here, as the hits come, so a run that ends badly
                        // still counts them
                        // a worktree's hits go to its project's journal, named by the card: the
                        // worktree's own went with it when the card landed, and no look-back saw them
                        if let Some(workspace) = &request.workspace {
                            let (root, card) = project_of(workspace);
                            if let Err(e) = Journal::at(&root.join("var/gotchas.log")).record(card, place, &hit) {
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
            let listed = turns.cost(&request.model);
            with_stream(parse_result(&result), |completion| {
                // a result without a cost, as a run out of turns can end, is priced from its turns
                if completion.usage.cost_usd.is_none() && listed.is_some() {
                    completion.usage.cost_usd = listed;
                    completion.usage.cost_computed = true;
                }
                completion.gotchas = gotchas;
                completion.read_in = turns.read_in();
                completion.whole_reads = turns.whole_reads();
                completion.turn_usage = turns.into_turns();
                completion.reads = reads;
            })
        } else if let Some((size, cap)) = crossed.zip(request.max_context) {
            let mut completion = turns.completion(&request.model);
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
    /// The share of each turn's input written to the cache, which is priced at its own rate.
    written: Vec<u64>,
    /// The text blocks of the last turn, the worker's words when the run ends without a result.
    said: Vec<String>,
    /// The index of the turn that made the first edit or write, once one has.
    first_edit: Option<usize>,
    /// The files read whole, with no range, as the calls named them.
    whole: std::collections::BTreeSet<String>,
}

/// The tools whose call is the run acting rather than reading in.
const EDIT_TOOLS: [&str; 4] = ["Edit", "Write", "MultiEdit", "NotebookEdit"];

/// What a turn line starts with: the sink reads the turns and the context off it instead of
/// showing it as a step.
pub const TURN_MARK: &str = "\u{23f1} turns ";

/// The progress line of a run `turns` turns in, the last of which read `input` tokens.
#[must_use]
pub fn turn_line(turns: usize, input: u64) -> String {
    format!("{TURN_MARK}{turns} \u{b7} {input} tokens")
}

/// The turns and the last turn's input a [`turn_line`] carries; `None` for any other line.
#[must_use]
pub fn parse_turn_line(text: &str) -> Option<(u32, u64)> {
    let (turns, input) = text.strip_prefix(TURN_MARK)?.split_once(" \u{b7} ")?;
    Some((turns.trim().parse().ok()?, input.strip_suffix(" tokens")?.trim().parse().ok()?))
}

impl TurnTally {
    /// Books one stream event, and gives the turn line when it began a new turn.
    fn turn_progress(&mut self, line: &str) -> Option<String> {
        let before = self.turns.len();
        self.note(line);
        if self.turns.len() > before { Some(turn_line(self.turns.len(), self.last_input()?)) } else { None }
    }

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
                if let Some(written) = self.written.last_mut() {
                    *written = count("cache_creation_input_tokens");
                }
                self.said.extend(text);
            }
            _ => {
                self.turns.push((id, turn));
                self.cached.push(count("cache_read_input_tokens"));
                self.written.push(count("cache_creation_input_tokens"));
                self.said = text.collect();
            }
        }
        // the calls of the turn: a whole read is context every later turn carries, and the
        // first edit ends the reading-in
        let calls = event
            .pointer("/message/content")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter(|block| block.get("type").and_then(serde_json::Value::as_str) == Some("tool_use"));
        for call in calls {
            let name = call.get("name").and_then(serde_json::Value::as_str).unwrap_or("");
            let input = call.get("input").unwrap_or(&serde_json::Value::Null);
            if name == "Read"
                && input.get("offset").is_none()
                && input.get("limit").is_none()
                && let Some(path) = input.get("file_path").and_then(serde_json::Value::as_str)
            {
                self.whole.insert(path.replace('\\', "/"));
            }
            if EDIT_TOOLS.contains(&name) && self.first_edit.is_none() {
                self.first_edit = Some(self.turns.len().saturating_sub(1));
            }
        }
    }

    /// The read-in: the input of the turns through the first edit, or of every turn when none
    /// edited.
    fn read_in(&self) -> u64 {
        let through = self.first_edit.map_or(self.turns.len(), |i| i + 1);
        self.turns.iter().take(through).map(|(_, turn)| turn.input_tokens).sum()
    }

    /// How many distinct files were read whole.
    fn whole_reads(&self) -> u64 {
        u64::try_from(self.whole.len()).unwrap_or(u64::MAX)
    }

    /// The input of the last turn, when the stream showed one.
    fn last_input(&self) -> Option<u64> {
        self.turns.last().map(|(_, turn)| turn.input_tokens)
    }

    /// The list price of the turns the stream showed on `model`, each side of the input at its
    /// own rate; `None` for a model the price table does not know or a stream without turns.
    fn cost(&self, model: &str) -> Option<f64> {
        let price = crate::openrouter::anthropic_price(model).filter(|_| !self.turns.is_empty())?;
        let tokens =
            self.turns.iter().zip(&self.cached).zip(&self.written).fold(Tokens::default(), |mut sum, (((_, turn), &cached), &written)| {
                sum.fresh += turn.input_tokens.saturating_sub(cached + written);
                sum.cache_read += cached;
                sum.cache_write += written;
                sum.output += turn.output_tokens;
                sum
            });
        Some(price.of(&tokens))
    }

    /// The completion of a run stopped before its result: the usage summed over the turns the
    /// stream showed, at the list price of `model` the ledger marks as computed, and the last
    /// turn's words.
    fn completion(self, model: &str) -> Completion {
        let turns = u64::try_from(self.turns.len()).unwrap_or(u64::MAX);
        let cost_usd = self.cost(model);
        let usage = Usage {
            prompt_tokens: self.turns.iter().map(|(_, turn)| turn.input_tokens).sum(),
            completion_tokens: self.turns.iter().map(|(_, turn)| turn.output_tokens).sum(),
            cached_tokens: self.cached.iter().sum(),
            cost_usd,
            cost_computed: cost_usd.is_some(),
        };
        let (read_in, whole_reads) = (self.read_in(), self.whole_reads());
        Completion {
            text: self.said.join("\n"),
            usage,
            turns,
            turn_usage: self.into_turns(),
            tool_calls: Vec::new(),
            session_id: None,
            gotchas: Vec::new(),
            reads: Vec::new(),
            read_in,
            whole_reads,
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

/// The turns of a resumed run the cap leaves alone: enough to read its verdict and answer it,
/// not the twelve of a fresh read-in. A resumed session already past the cap paid twelve turns
/// at twice the cap before (runelite #21: thirteen turns at 300k tokens each, $3.75, 2026-11-12);
/// now it hands off after two and a fresh session opens on the handoff.
pub const RESUMED_TURNS: usize = 2;

/// Watches a stream for the turn whose input passes the context cap, past the reading-in
/// turns. The run stops after that turn, once its tool results are in, so an edit in flight
/// lands and the handoff can name it; a result that comes first means the run finished on its
/// own and stands.
struct ContextWatch {
    cap: Option<u64>,
    /// The turns before the cap applies: [`READING_IN_TURNS`] fresh, [`RESUMED_TURNS`] resumed.
    grace: usize,
    /// The input of the turn that crossed the cap, once one has.
    crossed: Option<u64>,
}

impl ContextWatch {
    fn new(cap: Option<u64>) -> Self {
        Self { cap, grace: READING_IN_TURNS, crossed: None }
    }

    /// The watch of a resumed run: the cap applies after [`RESUMED_TURNS`].
    fn resumed(mut self, resumed: bool) -> Self {
        if resumed {
            self.grace = RESUMED_TURNS;
        }
        self
    }

    /// Reads one event after the tally has: the size of the crossing turn once the run may
    /// stop, `None` while it goes on.
    fn note(&mut self, line: &str, turns: &TurnTally) -> Option<u64> {
        let cap = self.cap?;
        if line.contains(r#""type":"assistant""#) {
            if turns.turns.len() > self.grace
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
/// relative to the workspace, with `:<from>-<to>` after it for a ranged read (the 1-based lines
/// the call asked for, the harness's 2000 when it named no limit), so a handoff can name the
/// regions the session read. Nothing from any other event.
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
        .filter_map(|block| {
            let input = block.get("input")?;
            let path = relative(input.get("file_path").and_then(Value::as_str)?, workspace);
            let offset = input.get("offset").and_then(Value::as_u64);
            let limit = input.get("limit").and_then(Value::as_u64);
            if offset.is_none() && limit.is_none() {
                return Some(path);
            }
            let from = offset.unwrap_or(1).max(1);
            let to = from + limit.unwrap_or(2000).max(1) - 1;
            Some(format!("{path}:{from}-{to}"))
        })
        .collect()
}

/// A path inside the workspace relative to it, with forward slashes; another path as it came.
fn relative(path: &str, workspace: Option<&Path>) -> String {
    // the workspace as given and made absolute: a worktree's is relative (`var/worktrees/47`)
    // while the tool's path is absolute, and the line showed the whole path
    let roots = workspace.into_iter().flat_map(|root| [Some(root.to_path_buf()), std::path::absolute(root).ok()]).flatten();
    let shown = roots
        .filter_map(|root| Path::new(path).strip_prefix(&root).ok().map(|p| p.to_string_lossy().into_owned()))
        .next()
        .unwrap_or_else(|| path.to_owned());
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
        read_in: 0,
        whole_reads: 0,
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
    // absolute: the harness runs in the workspace, and a relative path (a worktree's,
    // `var/worktrees/47`) read against it doubled into `var/worktrees/47/var/worktrees/47/…`
    std::path::absolute(&path).or(Ok(path))
}

/// `path` with `own`, the directory of this binary and the installed `rc` beside it, first and
/// the `tools` directories after the rest; a directory already on it stays where it is (on
/// Windows compared without case or a trailing separator). `None` when it cannot be joined.
fn path_with(own: Option<PathBuf>, tools: &[PathBuf], path: &std::ffi::OsStr) -> Option<std::ffi::OsString> {
    let key = |dir: &Path| {
        let text = dir.to_string_lossy();
        let text = text.trim_end_matches(['\\', '/']);
        if cfg!(windows) { text.to_lowercase() } else { text.to_owned() }
    };
    let mut dirs: Vec<PathBuf> = std::env::split_paths(path).collect();
    let mut seen: Vec<String> = dirs.iter().map(|d| key(d)).collect();
    if let Some(own) = own
        && !seen.contains(&key(&own))
    {
        seen.push(key(&own));
        dirs.insert(0, own);
    }
    for dir in tools {
        if !seen.contains(&key(dir)) {
            seen.push(key(dir));
            dirs.push(dir.clone());
        }
    }
    std::env::join_paths(dirs).ok()
}

/// The target directory for a run in one of the project's worktrees
/// (`<root>/var/worktrees/<id>`): `<root>/target/wt-<id>`, the worktree's own under the
/// project's target, so a build of one card never reads another's artifacts (two worktrees
/// editing one crate shared a target and each compiled against the other's copy, #176 and
/// #181), while the project's target stays the one place the builds live. The orchestrator's
/// check of the card builds in the same one, so its artifacts are warm and it never queues
/// behind the project's target. `None` for a workspace that is not a worktree.
#[must_use]
pub fn shared_target(workspace: &Path) -> Option<PathBuf> {
    let worktree = || -> Option<(PathBuf, String)> {
        let id = workspace.parent()?;
        if id.file_name()? != "worktrees" {
            return None;
        }
        let var = id.parent()?;
        if var.file_name()? != "var" {
            return None;
        }
        Some((var.parent()?.to_path_buf(), workspace.file_name()?.to_string_lossy().into_owned()))
    };
    match worktree() {
        Some((root, card)) => std::path::absolute(project_target(&root).join(format!("wt-{card}"))).ok(),
        // the project's own tree builds in its own target, on the scratch drive when there is one
        None => scratch().map(|_| project_target(workspace)),
    }
}

/// The machine's scratch root: where the heavy, throwaway output goes (build targets, temp
/// directories, the Gradle cache). `RHIPPER_SCRATCH` when set, else the hub's `scratch.txt`
/// that `rc scratch <dir>` writes. `None` keeps everything in the project as before.
#[must_use]
pub fn scratch() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("RHIPPER_SCRATCH").filter(|s| !s.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    let hub = match std::env::var_os("RHIPPER_HUB").filter(|h| !h.is_empty()) {
        Some(hub) => PathBuf::from(hub),
        None if cfg!(windows) => PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("rhipper").join("agent"),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".local").join("share").join("rhipper").join("agent"),
    };
    let text = std::fs::read_to_string(hub.join(SCRATCH_FILE)).ok()?;
    let dir = text.trim();
    (!dir.is_empty()).then(|| PathBuf::from(dir))
}

/// The hub's file that names the scratch root.
pub const SCRATCH_FILE: &str = "scratch.txt";

/// The cargo target of a project: `<scratch>/target/<project>` with a scratch root, else
/// `<root>/target`.
#[must_use]
pub fn project_target(root: &Path) -> PathBuf {
    match scratch() {
        Some(scratch) => {
            scratch.join("target").join(root.file_name().map_or_else(|| "project".into(), |n| n.to_string_lossy().into_owned()))
        }
        None => root.join("target"),
    }
}

/// The project a workspace belongs to and the card whose worktree it is: a worktree's project is
/// three levels up (`<root>/var/worktrees/<id>`), any other workspace is its own with no card.
#[must_use]
pub fn project_of(workspace: &Path) -> (PathBuf, Option<i64>) {
    if workspace.parent().and_then(Path::file_name).is_some_and(|n| n == "worktrees")
        && let Some(root) = workspace.ancestors().nth(3)
    {
        let card = workspace.file_name().and_then(|n| n.to_str()).and_then(|n| n.parse().ok());
        // a relative worktree (`var/worktrees/47`) has an empty root: the current directory
        let root = if root.as_os_str().is_empty() { Path::new(".") } else { root };
        return (root.to_path_buf(), card);
    }
    (workspace.to_path_buf(), None)
}

/// The environment a run or a check gets from the scratch root: its temp directory (the tests'
/// leftovers land there, not on the system drive) and the Gradle cache. Empty without one.
#[must_use]
pub fn scratch_env(workspace: &Path) -> Vec<(String, PathBuf)> {
    let Some(scratch) = scratch() else { return Vec::new() };
    let (root, _) = project_of(workspace);
    let name = root.file_name().map_or_else(|| "project".into(), |n| n.to_string_lossy().into_owned());
    let temp = scratch.join("tmp").join(name);
    let _ = std::fs::create_dir_all(&temp);
    vec![("TEMP".into(), temp.clone()), ("TMP".into(), temp), ("GRADLE_USER_HOME".into(), scratch.join("gradle"))]
}

/// The temp directory of a run in a workspace, absolute: `var/tmp` under it; none for a chat
/// completion, which runs nothing.
fn run_temp(request: &CompletionRequest) -> Option<PathBuf> {
    request.workspace.as_deref().and_then(|dir| std::path::absolute(dir.join(RUN_TEMP)).ok())
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
    conditions: Conditions<'_>,
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
                let content = text_of(block.get("content").unwrap_or(&Value::Null));
                let place = match tool {
                    Some("Bash" | "PowerShell") => Where::Result,
                    Some("Edit" | "Write" | "MultiEdit" | "NotebookEdit") => Where::Edit,
                    // a Read the pre hook refused comes back an error with the reason; a file's
                    // own text never does
                    Some("Read") if block.get("is_error").and_then(Value::as_bool) == Some(true) => {
                        hits.extend(catalogue.refused_read(&content));
                        continue;
                    }
                    _ => continue,
                };
                hits.extend(catalogue.scan(place, &content));
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

/// What the pre hook does with a Read `call` of the harness's hook input, in `root`: the
/// catalogue's [`Catalogue::read`] on the file, its line count, whether the call names a range,
/// and the items of the file when `context`, the card's context file, outlines it. A call that
/// is no Read, or a file that cannot be read, passes.
#[must_use]
pub fn read_verdict(catalogue: &Catalogue, root: &Path, context: &str, call: &serde_json::Value) -> Verdict {
    use serde_json::Value;
    if call.get("tool_name").and_then(Value::as_str) != Some("Read") {
        return Verdict::Pass;
    }
    let Some(input) = call.get("tool_input") else { return Verdict::Pass };
    let Some(file) = input.get("file_path").and_then(Value::as_str) else { return Verdict::Pass };
    let ranged = ["offset", "limit"].iter().any(|key| input.get(key).is_some_and(|v| !v.is_null()));
    let path = if Path::new(file).is_absolute() { PathBuf::from(file) } else { root.join(file) };
    let shown = relative(file, Some(root));
    // the outline names a file as the card does, relative and with forward slashes
    if ranged || !ra_context::outlined(context).contains(&shown) {
        return Verdict::Pass;
    }
    let Ok(text) = std::fs::read_to_string(&path) else { return Verdict::Pass };
    let items = ra_context::item_ranges(Path::new(&shown), &text);
    catalogue.read(ra_gotchas::ReadCall { file: &shown, ranged, lines: text.lines().count(), items: Some(&items) })
}

/// The `--settings` JSON that routes the harness's tool hooks to `rc hook pre` and `rc hook
/// post`, so a command the catalogue knows is refused or rewritten before it runs, a whole-file
/// Read of a long file the card outlines is refused with its items, a result it knows gets its
/// hint right after, and any other whole-file Read is answered with the outline, with no model
/// call.
fn hook_settings(rc: &Path) -> String {
    // the JSON encoder escapes the backslashes of a Windows path itself
    let command = |event: &str| format!("\"{}\" hook {event}", rc.display());
    serde_json::json!({
        "hooks": {
            "PreToolUse": [{"matcher": "Bash|PowerShell|Read", "hooks": [{"type": "command", "command": command("pre"), "timeout": 10}]}],
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
