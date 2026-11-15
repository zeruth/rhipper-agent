//! The verdicts of a landed tree: one a test of each card's hidden check, read from cargo's own
//! output, and the six checks of the tree as a whole. A hidden check is copied into its crate's
//! tests directory and run as its own test binary, so a card that never landed fails alone.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::Bundle;
use crate::pin::{Card, hidden_path};
use crate::project::DIR;

/// Where the solved tree sits in the bundle: the files the eight cards change or add, as a clean
/// solution leaves them.
pub const SOLVED: &str = "solved/";

/// The six checks of a landed tree, in the order [`tree`] gives them.
pub const TREE_CHECKS: [&str; 6] = ["tests", "clippy", "fmt", "scope", "quiet", "whole"];

/// The top-level names of a project no check reads: what its `.gitignore` leaves out, and git's
/// own directory.
const IGNORED: [&str; 5] = [".git", "target", "var", "Cargo.lock", "rhipper.md"];

/// The attributes that silence a lint; the base has none, so a lint silenced is not one passed.
const LINT_ATTRIBUTES: [&str; 4] = ["#[allow(", "#![allow(", "#[expect(", "#![expect("];

/// What a line of a conflict left unresolved starts with.
const CONFLICT_MARKERS: [&str; 3] = ["<<<<<<<", "=======", ">>>>>>>"];

/// How one test or one check of the tree ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// It ran and passed.
    Pass,
    /// It ran and failed, or it was not among the tests that ran.
    Fail,
    /// Its file did not compile, so no test of it ran.
    Compile,
}

impl Verdict {
    /// The verdict as the record writes it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "ok",
            Self::Fail => "fail",
            Self::Compile => "compile",
        }
    }
}

/// One verdict of a card's hidden check: the test by name and how it ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestVerdict {
    pub name: String,
    pub verdict: Verdict,
}

/// One check of the tree: its name, how it ended and the paths at fault where the check names
/// them, so a failure can be charged to the cards whose files they are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeVerdict {
    pub check: &'static str,
    pub verdict: Verdict,
    pub paths: Vec<String>,
}

/// Writes the solved tree of `bundle` over the project at `dest`.
///
/// # Errors
/// When a directory or file cannot be written.
pub fn lay_solved(bundle: &Bundle, dest: &Path) -> io::Result<()> {
    for path in bundle.paths() {
        let Some(rel) = path.strip_prefix(SOLVED) else { continue };
        let Some(bytes) = bundle.get(path) else { continue };
        let target = dest.join(rel);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(target, bytes)?;
    }
    Ok(())
}

/// The names of the tests of a hidden check's `source`: each function after a `#[test]`.
#[must_use]
pub fn test_names(source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut marked = false;
    for line in source.lines().map(str::trim) {
        if line == "#[test]" {
            marked = true;
        } else if marked && let Some(rest) = line.strip_prefix("fn ") {
            names.push(rest.split('(').next().unwrap_or(rest).trim().to_owned());
            marked = false;
        }
    }
    names
}

/// One verdict for each of `names` from the standard output of a test binary: a pass for a
/// `test <name> ... ok` line, a failure for any other end, and every one a compile failure when
/// no test ran at all.
#[must_use]
pub fn read_verdicts(names: &[String], stdout: &str) -> Vec<TestVerdict> {
    let ran = stdout.lines().any(|line| line.starts_with("running "));
    let passed: BTreeSet<&str> = stdout.lines().filter_map(|line| line.strip_prefix("test ")?.strip_suffix(" ... ok")).collect();
    names
        .iter()
        .map(|name| {
            let verdict = if !ran {
                Verdict::Compile
            } else if passed.contains(name.as_str()) {
                Verdict::Pass
            } else {
                Verdict::Fail
            };
            TestVerdict { name: name.clone(), verdict }
        })
        .collect()
}

/// The verdicts of `card`'s hidden check from `bundle` on the project at `dir`, as [`run_hidden`]
/// gives them.
///
/// # Errors
/// When the bundle has no hidden check for the card, or as [`run_hidden`].
pub fn hidden(bundle: &Bundle, card: &Card, dir: &Path) -> io::Result<Vec<TestVerdict>> {
    let path = hidden_path(&card.key);
    let source = bundle.get(&path).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no {path} in the benchmark")))?;
    run_hidden(dir, card, source)
}

/// Copies `source` to `crates/<crate>/tests/hidden_<key>.rs` of the project at `dir`, runs that
/// test binary alone and takes the file away again: one verdict a test of the file.
///
/// # Errors
/// When the source is not UTF-8, the file cannot be written or removed, or cargo does not start.
pub fn run_hidden(dir: &Path, card: &Card, source: &[u8]) -> io::Result<Vec<TestVerdict>> {
    let text = std::str::from_utf8(source).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{}: {e}", card.key)))?;
    let target = format!("hidden_{}", card.key);
    let tests = dir.join("crates").join(&card.krate).join("tests");
    fs::create_dir_all(&tests)?;
    let path = tests.join(format!("{target}.rs"));
    fs::write(&path, source)?;
    let output = cargo(dir, &["test", "-p", &card.krate, "--test", &target]);
    fs::remove_file(&path)?;
    Ok(read_verdicts(&test_names(text), &String::from_utf8_lossy(&output?.stdout)))
}

/// The six checks of the project at `dir`, in the order of [`TREE_CHECKS`]: its own tests, clippy
/// with warnings denied, rustfmt, every changed path against the base project of `base` and the
/// files `files` of the cards, no lint attribute in a Rust file and no conflict marker anywhere.
///
/// # Errors
/// When cargo does not start or a file of the tree cannot be read.
pub fn tree(dir: &Path, base: &Bundle, files: &BTreeSet<String>) -> io::Result<Vec<TreeVerdict>> {
    let tests = cargo(dir, &["test", "--workspace", "-q"])?;
    let clippy = cargo(dir, &["clippy", "--workspace", "--all-targets", "--message-format=short", "--", "-D", "warnings"])?;
    let fmt = cargo(dir, &["fmt", "--all", "--check"])?;
    let tracked = tracked(dir)?;
    let stray = changed(base, &tracked)?.into_iter().filter(|path| !files.contains(path)).collect();
    let quiet = with_line(&tracked, is_rust, |line| LINT_ATTRIBUTES.iter().any(|a| line.contains(a)))?;
    let whole = with_line(&tracked, |_| true, |line| CONFLICT_MARKERS.iter().any(|m| line.starts_with(m)))?;
    let clippy_paths = clippy_paths(&String::from_utf8_lossy(&clippy.stderr));
    Ok(vec![
        ran("tests", &tests, Vec::new()),
        ran("clippy", &clippy, clippy_paths),
        ran("fmt", &fmt, Vec::new()),
        scanned("scope", stray),
        scanned("quiet", quiet),
        scanned("whole", whole),
    ])
}

/// Runs cargo in `dir` offline, with its own target directory so no other build is shared.
fn cargo(dir: &Path, args: &[&str]) -> io::Result<Output> {
    Command::new("cargo")
        .args(args)
        .current_dir(dir)
        .env("CARGO_TARGET_DIR", dir.join("target"))
        .env("CARGO_NET_OFFLINE", "true")
        .env("CARGO_TERM_COLOR", "never")
        .output()
}

fn ran(check: &'static str, output: &Output, paths: Vec<String>) -> TreeVerdict {
    let verdict = if output.status.success() { Verdict::Pass } else { Verdict::Fail };
    TreeVerdict { check, verdict, paths }
}

fn scanned(check: &'static str, paths: Vec<String>) -> TreeVerdict {
    let verdict = if paths.is_empty() { Verdict::Pass } else { Verdict::Fail };
    TreeVerdict { check, verdict, paths }
}

/// The Rust files at the head of clippy's short lines, `/`-separated.
fn clippy_paths(stderr: &str) -> Vec<String> {
    let paths: BTreeSet<String> =
        stderr.lines().filter_map(|line| line.find(".rs:").map(|end| line[..end + 3].replace('\\', "/"))).collect();
    paths.into_iter().collect()
}

fn is_rust(rel: &str) -> bool {
    Path::new(rel).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("rs"))
}

fn ignored(rel: &str) -> bool {
    IGNORED.contains(&rel.split('/').next().unwrap_or(rel))
}

/// Every file of the project at `dir` a check reads, by its `/`-separated path, in order.
fn tracked(dir: &Path) -> io::Result<Vec<(String, PathBuf)>> {
    let mut files = Vec::new();
    walk(dir, "", &mut files)?;
    files.sort();
    Ok(files)
}

fn walk(dir: &Path, prefix: &str, files: &mut Vec<(String, PathBuf)>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        if prefix.is_empty() && ignored(&rel) {
            continue;
        }
        if entry.file_type()?.is_dir() {
            walk(&entry.path(), &rel, files)?;
        } else {
            files.push((rel, entry.path()));
        }
    }
    Ok(())
}

/// The paths a landing added, changed or removed against the base project, which is what a diff
/// of the landed branch from the base commit lists.
fn changed(base: &Bundle, tracked: &[(String, PathBuf)]) -> io::Result<Vec<String>> {
    let base_files: BTreeMap<&str, &[u8]> =
        base.paths().filter_map(|path| Some((path.strip_prefix(DIR)?, base.get(path)?))).filter(|(rel, _)| !ignored(rel)).collect();
    let mut out = BTreeSet::new();
    for (rel, path) in tracked {
        if base_files.get(rel.as_str()).copied() != Some(fs::read(path)?.as_slice()) {
            out.insert(rel.clone());
        }
    }
    let present: BTreeSet<&str> = tracked.iter().map(|(rel, _)| rel.as_str()).collect();
    out.extend(base_files.keys().filter(|rel| !present.contains(*rel)).map(|rel| (*rel).to_owned()));
    Ok(out.into_iter().collect())
}

/// The files of `tracked` that `wanted` picks and that have a line `hit` holds for.
fn with_line(tracked: &[(String, PathBuf)], wanted: impl Fn(&str) -> bool, hit: impl Fn(&str) -> bool) -> io::Result<Vec<String>> {
    let mut out = Vec::new();
    for (rel, path) in tracked {
        if wanted(rel) && String::from_utf8_lossy(&fs::read(path)?).lines().any(&hit) {
            out.push(rel.clone());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
