//! The check a card gets when its acceptance names none. A full workspace test run outlives a
//! session on a large workspace, so the files of the card name the crates to test instead, and
//! the workspace run stays only for a card whose files name no crate.

use std::path::Path;

/// The check of a card whose files name no crate.
pub const WORKSPACE_TESTS: &str = "cargo test --workspace -q";

/// The programs an acceptance runs a check through. `git` is not one: a backticked `git log`
/// is a reference, and the tools run the read-only git commands by themselves.
pub const CHECK_PROGRAMS: [&str; 34] = [
    "cargo", "rustc", "python", "python3", "py", "pytest", "uv", "pip", "node", "npm", "npx", "pnpm", "yarn", "bun", "deno", "tsc", "go",
    "make", "cmake", "ctest", "ninja", "just", "gradle", "gradlew", "mvn", "dotnet", "java", "zig", "swift", "ruby", "bundle", "php",
    "composer", "bash",
];

/// The extensions of a script named by a relative path.
const SCRIPT_EXTENSIONS: [&str; 5] = ["ps1", "sh", "py", "bat", "cmd"];

/// Whether a backticked span is a command line: one line whose first token is a check program
/// or a relative script path.
#[must_use]
pub fn is_command(span: &str) -> bool {
    if span.lines().count() != 1 {
        return false;
    }
    let Some(first) = span.split_whitespace().next() else { return false };
    if CHECK_PROGRAMS.contains(&first) {
        return true;
    }
    let is_script = Path::new(first).extension().and_then(|e| e.to_str()).is_some_and(|e| SCRIPT_EXTENSIONS.contains(&e));
    let is_relative = !first.starts_with('/') && !first.starts_with('\\') && !first.contains(':');
    is_script && is_relative
}

/// The directories a crate keeps its sources under; the component before one of them is the
/// crate's directory, and its name is the package name by the workspace's convention.
const SOURCE_DIRS: [&str; 4] = ["src", "tests", "benches", "examples"];

/// The tests of every crate `files` touch, one `cargo test -p <crate> -q` each in the order the
/// crates first appear, else the workspace tests.
#[must_use]
pub fn commands(files: &[String]) -> Vec<String> {
    let mut crates: Vec<&str> = Vec::new();
    for name in files.iter().filter_map(|file| crate_of(file)) {
        if !crates.contains(&name) {
            crates.push(name);
        }
    }
    if crates.is_empty() {
        return vec![WORKSPACE_TESTS.to_owned()];
    }
    crates.into_iter().map(|name| format!("cargo test -p {name} -q")).collect()
}

/// The crate a path lies in: the directory above its `src` (or `tests`, `benches`, `examples`),
/// provided that directory is not the workspace root itself.
#[must_use]
pub fn crate_of(file: &str) -> Option<&str> {
    let parts: Vec<&str> = file.split(['/', '\\']).filter(|part| !part.is_empty()).collect();
    let source = parts.iter().position(|part| SOURCE_DIRS.contains(part))?;
    let name = *parts.get(source.checked_sub(1)?)?;
    // a name with an extension is a file, not a directory
    (Path::new(name).extension().is_none()).then_some(name)
}
