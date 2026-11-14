//! A check after a merge from main rebuilds the card's own crates. Cargo judges an artifact fresh
//! by mtimes, and a merge can bring sources older than the worktree's last build: the check then
//! linked a stale rlib. A check with no merge in between stays incremental.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The file in a worktree's target that names the HEAD its last check built.
const STAMP: &str = ".check-head";

/// The crates the commands name with `-p`, `--package` or `-p=`, each once.
#[must_use]
pub fn crates_of(commands: &[String]) -> Vec<String> {
    let mut crates: Vec<String> = Vec::new();
    for command in commands {
        let mut words = command.split_whitespace();
        while let Some(word) = words.next() {
            let name = match word {
                "-p" | "--package" => words.next(),
                _ => word.strip_prefix("--package=").or_else(|| word.strip_prefix("-p=")),
            };
            if let Some(name) = name.map(|n| n.trim_matches(['"', '\''])).filter(|n| !n.is_empty())
                && !crates.iter().any(|c| c == name)
            {
                crates.push(name.to_owned());
            }
        }
    }
    crates
}

/// Whether a command is a cargo one that names no crate (`cargo test`, `cargo test --workspace`):
/// it builds the package of the directory or the whole workspace.
fn names_no_crate(command: &str) -> bool {
    let is_cargo = command.split_whitespace().next().is_some_and(|w| w.trim_end_matches(".exe") == "cargo");
    is_cargo && crates_of(&[command.to_owned()]).is_empty()
}

/// The packages of the workspace at `dir`, by `cargo metadata`; none when it cannot be read.
fn members(dir: &Path) -> Vec<String> {
    let Some(out) = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1", "-q"])
        .current_dir(dir)
        .output()
        .ok()
        .filter(|out| out.status.success())
    else {
        return Vec::new();
    };
    let Ok(metadata) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else { return Vec::new() };
    metadata["packages"].as_array().into_iter().flatten().filter_map(|package| package["name"].as_str().map(str::to_owned)).collect()
}

/// A rebuild done before a check: the crates whose fingerprints went, and the HEAD to record once
/// the check has run.
#[derive(Debug)]
#[must_use = "the HEAD is recorded only by `seen`"]
pub struct Rebuild {
    /// The crates whose fingerprints went; none when no merge came in between.
    pub cleaned: Vec<String>,
    stamp: PathBuf,
    head: Option<String>,
}

impl Rebuild {
    /// Records the HEAD this check built, so the next check without a merge stays incremental.
    /// Called after the check's commands ran: an interrupted check leaves the last HEAD, and the
    /// next one cleans again, slower but never stale.
    ///
    /// # Errors
    /// When the stamp cannot be written.
    pub fn seen(self) -> std::io::Result<()> {
        let Some(head) = self.head else { return Ok(()) };
        if let Some(dir) = self.stamp.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&self.stamp, head)
    }
}

/// Marks the card's crates stale in `target` when the worktree's HEAD has moved by a merge since
/// the last recorded check, so the next `cargo test` rebuilds them. The crates are those the
/// `commands` name, and every workspace member when a cargo command names none. Nothing is
/// cleaned on a first check, an unchanged HEAD or a HEAD moved by commits of the card's own.
///
/// # Errors
/// When a fingerprint cannot be removed.
pub fn rebuild_after_merge(worktree: &Path, target: &Path, commands: &[String]) -> std::io::Result<Rebuild> {
    let head = git(worktree, &["rev-parse", "HEAD"]);
    let stamp = target.join(STAMP);
    let mut rebuild = Rebuild { cleaned: Vec::new(), stamp, head };
    let Some(head) = rebuild.head.as_deref() else { return Ok(rebuild) };
    let before = std::fs::read_to_string(&rebuild.stamp).ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
    let Some(before) = before.filter(|b| b != head) else { return Ok(rebuild) };
    // a HEAD that cannot be compared to the last one (a rewritten history) is cleaned too: slower, never stale
    let merges = git(worktree, &["rev-list", "--merges", "--count", &format!("{before}..{head}")]);
    if merges.as_deref() == Some("0") {
        return Ok(rebuild);
    }
    let mut crates = crates_of(commands);
    if commands.iter().any(|c| names_no_crate(c)) {
        for name in members(worktree) {
            if !crates.contains(&name) {
                crates.push(name);
            }
        }
    }
    for name in crates {
        if forget(target, &name)? {
            rebuild.cleaned.push(name);
        }
    }
    Ok(rebuild)
}

/// Removes the fingerprints of `name` from every profile of `target`; whether any was there.
fn forget(target: &Path, name: &str) -> std::io::Result<bool> {
    let prefix = format!("{name}-");
    let mut found = false;
    let Ok(profiles) = std::fs::read_dir(target) else { return Ok(false) };
    for profile in profiles.flatten() {
        let Ok(prints) = std::fs::read_dir(profile.path().join(".fingerprint")) else { continue };
        for print in prints.flatten() {
            let file = print.file_name();
            // `<name>-<16 hex digits>`, so `ra-console` never takes `ra-console-extra`'s
            let is_it = file
                .to_string_lossy()
                .strip_prefix(&prefix)
                .is_some_and(|hash| hash.len() == 16 && hash.bytes().all(|b| b.is_ascii_hexdigit()));
            if is_it {
                std::fs::remove_dir_all(print.path())?;
                found = true;
            }
        }
    }
    Ok(found)
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).current_dir(dir).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}
