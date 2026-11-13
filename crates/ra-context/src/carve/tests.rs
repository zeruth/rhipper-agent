use super::*;

const STORE: &str = r#"//! The store over a file.

use std::fmt::Write;
use std::path::Path;

use anyhow::Context;

/// How many lines a parse keeps.
const ROOM: usize = 3;

/// Opens the store at `path`.
pub fn open(path: &Path) -> anyhow::Result<String> {
    std::fs::read_to_string(path).context("cannot read the store")
}

/// The lines of a store, `ROOM` at most.
#[must_use]
pub fn parse(text: &str) -> Vec<&str> {
    // a `}` in a comment or "a { string" closes nothing
    text.lines().take(ROOM).collect()
}

/// The store as one line each.
fn render(lines: &[&str]) -> String {
    let mut out = String::new();
    for line in lines {
        let _ = writeln!(out, "{line}");
    }
    out
}

pub fn summary(path: &Path) -> anyhow::Result<String> {
    Ok(render(&parse(&open(path)?)))
}

#[cfg(test)]
mod tests;
"#;

fn read_from(dir: &Path) -> impl Fn(&str) -> Option<String> + '_ {
    move |file| std::fs::read_to_string(dir.join(file)).ok()
}

#[test]
fn carve_moves_two_functions_with_their_docs_and_the_dry_run_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/store.rs"), STORE).unwrap();
    let names = ["parse".to_owned(), "render".to_owned()];

    // the dry run: what would move, and no file touched
    let carve = carve(read_from(dir.path()), "src/store.rs", "src/store/parse.rs", &names).unwrap();
    assert_eq!(carve.moved.iter().map(|(name, ..)| name.as_str()).collect::<Vec<_>>(), ["parse", "render"]);
    assert_eq!(std::fs::read_to_string(dir.path().join("src/store.rs")).unwrap(), STORE);
    assert!(!dir.path().join("src/store").exists());

    write(dir.path(), "src/store.rs", "src/store/parse.rs", &carve).unwrap();
    let new = std::fs::read_to_string(dir.path().join("src/store/parse.rs")).unwrap();
    let old = std::fs::read_to_string(dir.path().join("src/store.rs")).unwrap();

    // both functions moved whole, their doc comments and attributes with them
    assert!(new.contains("/// The lines of a store, `ROOM` at most.\n#[must_use]\npub fn parse(text: &str) -> Vec<&str> {\n"));
    assert!(new.contains("    text.lines().take(ROOM).collect()\n}\n"));
    assert!(new.contains("/// The store as one line each.\npub(super) fn render(lines: &[&str]) -> String {\n"));
    assert!(new.contains("    out\n}\n"));
    // under the imports they use and the old file's items they name, and none they do not
    assert!(new.starts_with("use std::fmt::Write;\nuse super::ROOM;\n\n"), "{new}");
    assert!(!new.contains("use std::path::Path;"));
    assert!(!new.contains("anyhow"));

    // the old file keeps the rest, a mod line and the re-exports, so no caller changes
    assert!(!old.contains("fn parse"));
    assert!(!old.contains("fn render"));
    assert!(!old.contains("The lines of a store"));
    assert!(old.contains("mod parse;\npub use parse::parse;\nuse parse::render;\n"), "{old}");
    assert!(old.contains("pub fn open(path: &Path)"));
    assert!(old.contains("pub fn summary(path: &Path)"));
    assert!(old.contains("const ROOM: usize = 3;"));
    assert!(old.ends_with("\n#[cfg(test)]\nmod tests;\n"));
    // the import only the moved items used is gone; the ones the rest uses stay
    assert!(!old.contains("use std::fmt::Write;"));
    assert!(old.contains("use std::path::Path;"));
    assert!(old.contains("use anyhow::Context;"));
    assert!(!old.contains("\n\n\n"));
}

#[test]
fn carve_moves_nothing_for_a_name_the_file_does_not_declare() {
    let read = |file: &str| (file == "src/store.rs").then(|| STORE.to_owned());
    let error = carve(read, "src/store.rs", "src/store/parse.rs", &["missing".to_owned()]).unwrap_err();
    assert!(error.to_string().contains("missing is not a top-level item"));
    // a file a `mod` line in the old one would not find is refused
    assert!(carve(read, "src/store.rs", "src/parse.rs", &["parse".to_owned()]).is_err());
    assert!(carve(read, "src/lib.rs", "src/parse.rs", &["parse".to_owned()]).is_err());
}
