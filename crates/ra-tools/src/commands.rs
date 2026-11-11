//! The commands of the toolbox: the allowlist, the run under the time limit, and the clip that
//! keeps a long output from carrying whole into a turn.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::Context;

use crate::Toolbox;

/// The most lines a tool answer carries into the turn; a longer one keeps its head and its tail.
pub(crate) const MAX_LINES: usize = 200;
/// The most characters a tool answer carries; a longer one is cut and says so.
pub(crate) const MAX_OUTPUT: usize = 16_000;

impl Toolbox {
    /// Runs an allowlisted command in the workspace under the toolbox's time limit, for a check:
    /// whether it exited with code 0, and its whole output ending with the exit line. The output
    /// is not clipped: the lint reads every diagnostic of it.
    ///
    /// # Errors
    /// When the command is off the allowlist, cannot start or runs past the limit.
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
    /// When the command is empty, cannot start or runs past the limit.
    pub async fn check_acceptance(&self, command: &str) -> anyhow::Result<(bool, String)> {
        let text = self.run(command, false).await?;
        let passed = text.lines().last() == Some("exit code 0");
        Ok((passed, text))
    }

    pub(crate) async fn run(&self, command: &str, allowlisted: bool) -> anyhow::Result<String> {
        let mut parts = command.split_whitespace();
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
            let child =
                tokio::process::Command::new(program).args(&args).current_dir(&self.root).stdin(std::process::Stdio::null()).output();
            let output = tokio::time::timeout(self.timeout, child)
                .await
                .map_err(|_| anyhow::anyhow!("{program} ran past {} s and was stopped", self.timeout.as_secs()))?
                .with_context(|| format!("cannot start {program}"))?;
            let mut text = String::new();
            text.push_str(&String::from_utf8_lossy(&output.stdout));
            if !output.stderr.is_empty() {
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(&String::from_utf8_lossy(&output.stderr));
            }
            (output.status.code(), text)
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
        format!("{}\n… {} lines clipped …\n{}", lines[..half].join("\n"), lines.len() - MAX_LINES, lines[lines.len() - half..].join("\n"))
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
