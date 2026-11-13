//! `rc check` names the real path when a card says `crates/rc` for the crate at `apps/rc`.

use std::path::Path;
use std::process::Command;

fn rc(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rc")).args(args).current_dir(dir).output().expect("rc runs")
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a tempdir");
    std::fs::create_dir_all(dir.path().join("apps/rc/src")).expect("the crate dir");
    std::fs::write(dir.path().join("apps/rc/Cargo.toml"), "[package]\nname = \"rc\"\n").expect("a manifest");
    std::fs::write(dir.path().join("apps/rc/src/main.rs"), "fn main() {}\n").expect("a source");
    assert!(rc(dir.path(), &["init", "--empty"]).status.success());
    assert!(rc(dir.path(), &["add", "project", "Alpha"]).status.success());
    dir
}

fn check_of(dir: &Path) -> String {
    let out = rc(dir, &["check"]);
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn card_check_crate_path_in_files_names_the_real_path() {
    let dir = project();
    let added = rc(dir.path(), &["add", "task", "Fix", "--parent", "1", "--files", "crates/rc/src/main.rs"]);
    assert!(added.status.success(), "{}", String::from_utf8_lossy(&added.stderr));
    let shown = check_of(dir.path());
    assert!(shown.contains("crates/rc/ is not in the tree; the crate is apps/rc/"), "{shown}");
}

#[test]
fn card_check_crate_path_in_text_names_the_real_path_and_a_right_path_is_clean() {
    let dir = project();
    let added = rc(
        dir.path(),
        &["add", "task", "Fix", "--parent", "1", "--objective", "Change crates/rc/src/main.rs", "--files", "apps/rc/src/main.rs"],
    );
    assert!(added.status.success(), "{}", String::from_utf8_lossy(&added.stderr));
    let shown = check_of(dir.path());
    assert!(shown.contains("crates/rc/ is not in the tree; the crate is apps/rc/"), "{shown}");

    let right = project();
    rc(
        right.path(),
        &["add", "task", "Fix", "--parent", "1", "--objective", "Change apps/rc/src/main.rs", "--files", "apps/rc/src/main.rs"],
    );
    let shown = check_of(right.path());
    assert!(!shown.contains("is not in the tree"), "{shown}");
}
