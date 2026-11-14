//! A run whose card lists `+` files that already exist, with an acceptance that passes, starts no
//! worker and marks the card `already in base`; a card whose file is missing still goes to a worker.

use std::path::Path;
use std::process::Command;

fn rc(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rc")).args(args).current_dir(dir).output().expect("rc runs")
}

/// A project whose crate passes its tests, with one card that lists `+src/lib.rs`.
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a tempdir");
    std::fs::create_dir_all(dir.path().join("src")).expect("the source dir");
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n")
        .expect("a manifest");
    std::fs::write(dir.path().join("src/lib.rs"), "#[test]\nfn it_holds() {}\n").expect("a source");
    assert!(rc(dir.path(), &["init", "--empty"]).status.success());
    assert!(rc(dir.path(), &["add", "project", "Alpha"]).status.success());
    dir
}

fn add_card(dir: &Path, file: &str) {
    let added = rc(dir, &["add", "task", "Make", "--parent", "1", "--files", file, "--acceptance", "`cargo test -p probe`"]);
    assert!(added.status.success(), "{}", String::from_utf8_lossy(&added.stderr));
}

#[test]
fn a_card_whose_files_and_check_are_in_the_base_starts_no_worker() {
    let dir = project();
    add_card(dir.path(), "+src/lib.rs");
    let run = rc(dir.path(), &["run", "3"]);
    let said = String::from_utf8_lossy(&run.stdout).into_owned();
    assert!(said.contains("#3 already in base"), "{said}{}", String::from_utf8_lossy(&run.stderr));
}

#[test]
fn a_card_whose_file_is_missing_is_not_in_the_base() {
    let dir = project();
    add_card(dir.path(), "+src/other.rs");
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a runtime");
    let in_base = runtime.block_on(async {
        let store = ra_roadmap::store::Store::open(&dir.path().join("rhipper.md"), "Rhipper Agent").await.expect("the store");
        ra_console::agent::in_base(&store, 3).await.expect("the card reads")
    });
    assert!(!in_base);
}
