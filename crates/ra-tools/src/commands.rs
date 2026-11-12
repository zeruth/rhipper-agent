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
        let text = self.run(command, false).await?;
        let passed = text.lines().last() == Some("exit code 0");
        Ok((passed, text))
    }

    pub(crate) async fn run(&self, command: &str, allowlisted: bool) -> anyhow::Result<String> {
        let words = split(command)?;
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
                && let Some(target) = shared_target(&self.root)
            {
                command.env("CARGO_TARGET_DIR", target);
            }
            let mut child = command.spawn().with_context(|| format!("cannot start {program}"))?;
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
            let (out, err) = (stdout.await, stderr.await);
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

/// Reads a child's pipe to its end on its own task, and answers with what came, even when the
/// child was stopped before the end. A pipe the child did not open reads as empty.
fn drain<R>(pipe: Option<R>) -> impl std::future::Future<Output = Vec<u8>>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let task = pipe.map(|mut pipe| {
        tokio::spawn(async move {
            let mut buffer = Vec::new();
            let _ = tokio::io::AsyncReadExt::read_to_end(&mut pipe, &mut buffer).await;
            buffer
        })
    });
    async move {
        match task {
            Some(task) => task.await.unwrap_or_default(),
            None => Vec::new(),
        }
    }
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

/// The project's target directory for a workspace that is one of its worktrees
/// (`<root>/var/worktrees/<id>`); `None` for any other workspace.
fn shared_target(workspace: &Path) -> Option<PathBuf> {
    let id = workspace.parent()?;
    if id.file_name()? != "worktrees" {
        return None;
    }
    let var = id.parent()?;
    if var.file_name()? != "var" {
        return None;
    }
    std::path::absolute(var.parent()?.join("target")).ok()
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
