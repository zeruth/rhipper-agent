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

/// The `<16 hex digits>` hash a cargo artifact's name ends in, split from its stem.
fn split_hash(stem: &str) -> Option<(&str, &str)> {
    stem.rsplit_once('-').filter(|(_, hash)| hash.len() == 16 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// The crates a test binary links, by its fingerprint directory `print`: its own crate `name`, and
/// every dependency its `test-*.json` names. None when the fingerprint is not a test target's, so
/// a build script or a plain binary is never taken for a test.
fn test_links(print: &Path, name: &str) -> Option<Vec<String>> {
    let entries = std::fs::read_dir(print).ok()?;
    let mut test = false;
    let mut links = vec![name.to_owned()];
    for entry in entries.flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        if !file.starts_with("test-") {
            continue;
        }
        test = true;
        if file.ends_with(".json")
            && let Ok(text) = std::fs::read_to_string(entry.path())
            && let Ok(json) = serde_json::from_str::<serde_json::Value>(&text)
        {
            // `"deps": [[<hash>, "<extern name>", <public>, <fingerprint>], …]`
            let deps = json["deps"].as_array().into_iter().flatten();
            links.extend(deps.filter_map(|dep| dep.get(1)?.as_str().map(str::to_owned)));
        }
    }
    test.then_some(links)
}

/// Removes every test binary in `target` that links the library of one of `crates` and is older
/// than that library's rlib, with its fingerprint so cargo rebuilds it instead of running it.
/// Returns one line per binary, `<binary> rebuilt: stale against <lib>`. A merge can leave a
/// binary built before the rlib it links; its mtime is then no proof it is fresh. A binary that
/// links none of `crates`, and an artifact that is not a test binary, are left alone.
///
/// # Errors
/// When a stale binary or its fingerprint cannot be removed.
pub fn rebuild_stale_binaries(target: &Path, crates: &[String]) -> std::io::Result<Vec<String>> {
    let mut lines = Vec::new();
    let crates: Vec<String> = crates.iter().map(|c| c.replace('-', "_")).collect();
    let Ok(profiles) = std::fs::read_dir(target) else { return Ok(lines) };
    for profile in profiles.flatten() {
        let Ok(entries) = std::fs::read_dir(profile.path().join("deps")) else { continue };
        let entries: Vec<_> = entries.flatten().collect();
        // the newest rlib of each of the card's crates, by its extern name
        let mut libs: Vec<(String, std::time::SystemTime, String)> = Vec::new();
        for entry in &entries {
            let file = entry.file_name().to_string_lossy().into_owned();
            let Some(stem) = file.strip_prefix("lib").and_then(|f| f.strip_suffix(".rlib")) else { continue };
            let Some((name, _)) = split_hash(stem) else { continue };
            let Some(name) = crates.iter().find(|c| *c == name) else { continue };
            let Ok(built) = entry.metadata().and_then(|m| m.modified()) else { continue };
            match libs.iter_mut().find(|(n, ..)| n == name) {
                Some(lib) if built > lib.1 => *lib = (name.clone(), built, file),
                Some(_) => {}
                None => libs.push((name.clone(), built, file)),
            }
        }
        if libs.is_empty() {
            continue;
        }
        // the fingerprint directory is named by the package (`ra-providers-<hash>`), the binary by
        // the crate (`ra_providers-<hash>`)
        let prints: Vec<PathBuf> =
            std::fs::read_dir(profile.path().join(".fingerprint")).into_iter().flatten().flatten().map(|entry| entry.path()).collect();
        let print_of = |name: &str, hash: &str| {
            let wanted = format!("{name}-{hash}");
            prints.iter().find(|p| p.file_name().is_some_and(|f| f.to_string_lossy().replace('-', "_") == wanted.replace('-', "_")))
        };
        // gathered before any removal: the `.exe` and the bare binary share one fingerprint
        let mut stale: Vec<(PathBuf, &PathBuf, &str)> = Vec::new();
        for entry in &entries {
            let path = entry.path();
            if path.extension().is_some_and(|e| e != "exe") {
                continue;
            }
            let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else { continue };
            let Some((name, hash)) = split_hash(&stem) else { continue };
            let Some(print) = print_of(name, hash) else { continue };
            let Some(links) = test_links(print, name) else { continue };
            let Ok(built) = entry.metadata().and_then(|m| m.modified()) else { continue };
            let against = libs.iter().filter(|(lib, ..)| links.contains(lib)).filter(|(_, at, _)| built < *at).max_by_key(|(_, at, _)| *at);
            if let Some((.., lib_name)) = against {
                stale.push((path, print, lib_name.as_str()));
            }
        }
        for (path, print, lib_name) in stale {
            std::fs::remove_file(&path)?;
            if print.exists() {
                std::fs::remove_dir_all(print)?;
            }
            let binary = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            lines.push(format!("{binary} rebuilt: stale against {lib_name}"));
        }
    }
    Ok(lines)
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
