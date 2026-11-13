//! The commands of the toolbox: the allowlist, the run under the time limit, and the clip that
//! keeps a long output from carrying whole into a turn.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::Toolbox;

/// The most lines a tool answer carries into the turn; a longer one keeps its head and its tail.
/// A whole test or build log carried in would pass the ledger's ceiling on a turn by itself,
/// and every later turn would carry it again.
pub(crate) const MAX_LINES: usize = 200;
/// The most characters a tool answer carries; a longer one is cut and says so.
pub(crate) const MAX_OUTPUT: usize = 16_000;

impl Toolbox {
    /// Runs an allowlisted command in the workspace under the toolbox's time limit, for a check:
    /// whether it exited with code 0, and its whole output ending with the exit line. The output
    /// is not clipped: the lint reads every diagnostic of it. A command that runs past the limit
    /// is killed with the processes it started, and its output ends in `timed out after Ns`.
    ///
    /// # Errors
    /// When the command is off the allowlist or cannot start.
    pub async fn check(&self, command: &str) -> anyhow::Result<(bool, String)> {
        let text = self.run(command, true).await?;
        let passed = text.lines().last() == Some("exit code 0");
        Ok((passed, text))
    }

    /// Runs a command the person wrote in a card's acceptance, under the same limit and with
    /// the same answer as [`check`](Self::check), but past the allowlist: the acceptance names
    /// the project's own checks (a gradle wrapper, a python script), which a model never gets.
    /// A `.ps1`, `.py` or `.sh` program starts through its interpreter, since none of them is an
    /// executable on every platform.
    ///
    /// # Errors
    /// When the command is empty or cannot start.
    pub async fn check_acceptance(&self, command: &str) -> anyhow::Result<(bool, String)> {
        self.check_args(&words(command)?).await
    }

    /// Runs an acceptance command already split into its words, as
    /// [`check_acceptance`](Self::check_acceptance) does: the words reach the program as its
    /// arguments, never joined and split again, so a `.\gw.ps1` keeps its backslash (runelite
    /// #88 lost it to a second split). A script sees the words joined by spaces.
    ///
    /// # Errors
    /// When there are no words or the program cannot start.
    pub async fn check_args(&self, words: &[String]) -> anyhow::Result<(bool, String)> {
        let text = self.run_words(words, &words.join(" "), false).await?;
        let passed = text.lines().last() == Some("exit code 0");
        Ok((passed, text))
    }

    pub(crate) async fn run(&self, command: &str, allowlisted: bool) -> anyhow::Result<String> {
        self.run_words(&split(command)?, command, allowlisted).await
    }

    /// Runs `words` as a program and its arguments; `command` is the line a script answers.
    async fn run_words(&self, words: &[String], command: &str, allowlisted: bool) -> anyhow::Result<String> {
        let mut parts = words.iter().map(String::as_str);
        let program = parts.next().context("the command is empty")?;
        let mut args: Vec<&str> = parts.collect();
        anyhow::ensure!(
            !allowlisted || allowed(program, &args),
            "{program} is not allowed; the commands are cargo, and git status, diff, log and show"
        );
        let (code, mut text) = if let Some(script) = &self.script {
            let (code, text) = script(command)?;
            (Some(code), text)
        } else {
            let program = match interpreter_of(program) {
                Some((interpreter, leading)) => {
                    args.splice(0..0, leading.iter().copied().chain(std::iter::once(program)));
                    interpreter
                }
                None => program,
            };
            let mut command = tokio::process::Command::new(program);
            command
                .args(&args)
                .current_dir(&self.root)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true);
            // a check in a card's worktree builds in the project's target, as the worker did:
            // one build directory for the cards running at once, under the one lock
            if std::env::var_os("CARGO_TARGET_DIR").is_none()
                && let Some(target) = ra_providers::claude_code::shared_target(&self.root)
            {
                command.env("CARGO_TARGET_DIR", target);
            }
            // the scratch drive's temp directory and Gradle cache, when the machine has one
            command.envs(ra_providers::claude_code::scratch_env(&self.root));
            command.envs(self.env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
            let mut child = command.spawn().with_context(|| format!("cannot start {program}"))?;
            // the check is a build: it runs on its share of the builders' CPU, which follows
            // the number of builds running until it ends
            #[cfg(windows)]
            let _share = child.raw_handle().and_then(crate::cpu::attach);
            // the output is read into buffers the run keeps when the limit ends it, so a check
            // that outlives its budget still leaves its last lines
            let stdout = drain(child.stdout.take());
            let stderr = drain(child.stderr.take());
            let waited = tokio::time::timeout(self.timeout, child.wait()).await;
            let code = if let Ok(status) = waited {
                Ok(status.with_context(|| format!("cannot wait for {program}"))?.code())
            } else {
                kill_tree(&mut child).await;
                Err(())
            };
            // the command has exited, but a process it started may hold the pipes open for
            // ever: a gradle daemon spawned by the check inherits them, and a read to the end
            // never ended (the runelite walk hung on #82's landing for half an hour). What
            // came in the grace period after the exit is the output
            let (out, err) = (stdout.finish(PIPE_GRACE).await, stderr.finish(PIPE_GRACE).await);
            let mut text = String::new();
            text.push_str(&String::from_utf8_lossy(&out));
            if !err.is_empty() {
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(&String::from_utf8_lossy(&err));
            }
            if let Ok(code) = code {
                (code, text)
            } else {
                if !text.ends_with('\n') && !text.is_empty() {
                    text.push('\n');
                }
                write!(text, "timed out after {}s", self.timeout.as_secs()).expect("a String takes every write");
                return Ok(text);
            }
        };
        if !text.ends_with('\n') && !text.is_empty() {
            text.push('\n');
        }
        match code {
            Some(code) => write!(text, "exit code {code}").expect("a String takes every write"),
            None => text.push_str("stopped by a signal"),
        }
        Ok(text)
    }
}

/// How long a check's pipes are read after its command has exited: what a process it left
/// behind writes later is not the check's.
const PIPE_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

/// A child's pipe read on its own task into a buffer the caller can take at any time.
struct Drain {
    buffer: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl Drain {
    /// What the pipe gave: to its end, or what had come when `grace` ran out; the reading
    /// task is stopped then, so a pipe held open by another process costs nothing more.
    async fn finish(self, grace: std::time::Duration) -> Vec<u8> {
        if let Some(task) = self.task {
            let abort = task.abort_handle();
            if tokio::time::timeout(grace, task).await.is_err() {
                abort.abort();
            }
        }
        std::mem::take(&mut *self.buffer.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
    }
}

/// Reads a child's pipe on its own task into a shared buffer, so what came is kept when the
/// child is stopped or the pipe never ends. A pipe the child did not open reads as empty.
fn drain<R>(pipe: Option<R>) -> Drain
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let buffer = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let task = pipe.map(|mut pipe| {
        let buffer = buffer.clone();
        tokio::spawn(async move {
            let mut chunk = [0u8; 8192];
            loop {
                match tokio::io::AsyncReadExt::read(&mut pipe, &mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buffer.lock().unwrap_or_else(std::sync::PoisonError::into_inner).extend_from_slice(&chunk[..n]),
                }
            }
        })
    });
    Drain { buffer, task }
}

/// Stops a child that ran past its limit, and with it the processes it started: a `cargo test`
/// left alone would go on in its test binary after cargo itself was killed, holding the target
/// directory and the pipes. Windows kills the tree by `taskkill`; elsewhere the child alone.
async fn kill_tree(child: &mut tokio::process::Child) {
    if cfg!(windows)
        && let Some(pid) = child.id()
    {
        let _ = tokio::process::Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;
    }
    let _ = child.kill().await;
}

/// The words of a command line as a check runs them: the program, then its arguments, split
/// once by [`split`]'s quoting so the caller passes them on whole.
///
/// # Errors
/// When a quote is unbalanced or the line ends in a backslash.
pub fn words(command: &str) -> anyhow::Result<Vec<String>> {
    split(command)
}

/// The words of a command by shell quoting, not whitespace: a `"…"` or `'…'` group stays one
/// argument, a backslash outside single quotes keeps the next character when that is a space, a
/// quote or a backslash, and is a path separator otherwise (`.\gw.ps1 :task` names the script,
/// not `.gw.ps1`), and an unbalanced quote is refused by name rather than split into a command
/// that passes on nothing.
pub(crate) fn split(command: &str) -> anyhow::Result<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    let mut chars = command.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' | '\'' => {
                quoted = true;
                loop {
                    match chars.next() {
                        Some(end) if end == c => break,
                        Some('\\') if c == '"' => word.push(chars.next().context("the command ends in a backslash")?),
                        Some(inner) => word.push(inner),
                        None => anyhow::bail!("the command has an unbalanced {c} quote: {command}"),
                    }
                }
            }
            '\\' => match chars.clone().next() {
                Some(next) if next.is_whitespace() || matches!(next, '"' | '\'' | '\\') => {
                    word.push(next);
                    chars.next();
                }
                Some(_) => word.push('\\'),
                None => anyhow::bail!("the command ends in a backslash: {command}"),
            },
            c if c.is_whitespace() => {
                if quoted || !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
                quoted = false;
            }
            c => word.push(c),
        }
    }
    if quoted || !word.is_empty() {
        words.push(word);
    }
    Ok(words)
}

/// The interpreter a script program starts through, and the arguments before the script's
/// path; None for a program that starts by itself.
fn interpreter_of(program: &str) -> Option<(&'static str, &'static [&'static str])> {
    let extension = Path::new(program).extension()?.to_str()?;
    match extension {
        "ps1" => Some(("powershell", &["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])),
        "py" => Some(("python", &[])),
        "sh" => Some(("sh", &[])),
        _ => None,
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

/// A tool answer within the limits, by lines and then by characters; a long one keeps its head
/// and its tail, since a build log opens with the first error and closes with the verdict, and
/// says how much went between them.
pub(crate) fn clip(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let text = if lines.len() > MAX_LINES {
        let half = MAX_LINES / 2;
        format!("{}\n… {} lines dropped …\n{}", lines[..half].join("\n"), lines.len() - MAX_LINES, lines[lines.len() - half..].join("\n"))
    } else {
        text.to_owned()
    };
    let count = text.chars().count();
    if count <= MAX_OUTPUT {
        return text;
    }
    let head: String = text.chars().take(MAX_OUTPUT / 2).collect();
    let tail: String = text.chars().rev().take(MAX_OUTPUT / 2).collect::<Vec<_>>().into_iter().rev().collect();
    format!("{head}\n… {} characters cut …\n{tail}", count - MAX_OUTPUT)
}
