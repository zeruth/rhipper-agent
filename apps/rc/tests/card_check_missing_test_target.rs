//! `rc check` flags an acceptance that names a `--test` target no file provides, and names the
//! open card whose files list it.

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
    String::from_utf8_lossy(&rc(dir, &["check"]).stdout).into_owned()
}

const ACCEPTANCE: &str = "`cargo test -p rc --test gamma`";

#[test]
fn a_test_target_no_file_provides_is_flagged() {
    let dir = project();
    let added = rc(dir.path(), &["add", "task", "Fix", "--parent", "1", "--acceptance", ACCEPTANCE]);
    assert!(added.status.success(), "{}", String::from_utf8_lossy(&added.stderr));
    let shown = check_of(dir.path());
    assert!(shown.contains("--test gamma has no tests/gamma.rs"), "{shown}");
}

#[test]
fn the_report_names_the_open_card_that_lists_the_test() {
    let dir = project();
    rc(dir.path(), &["add", "task", "Needs", "--parent", "1", "--acceptance", ACCEPTANCE]);
    let other = rc(dir.path(), &["add", "task", "Makes", "--parent", "1", "--files", "+apps/rc/tests/gamma.rs"]);
    assert!(other.status.success(), "{}", String::from_utf8_lossy(&other.stderr));
    let shown = check_of(dir.path());
    assert!(shown.contains("#3: --test gamma has no tests/gamma.rs in the tree; #4 lists it"), "{shown}");
}

#[test]
fn a_test_in_the_tree_or_in_the_cards_own_files_is_clean() {
    let dir = project();
    std::fs::create_dir_all(dir.path().join("apps/rc/tests")).expect("a tests dir");
    std::fs::write(dir.path().join("apps/rc/tests/gamma.rs"), "").expect("a test");
    rc(dir.path(), &["add", "task", "Fix", "--parent", "1", "--acceptance", ACCEPTANCE]);
    let shown = check_of(dir.path());
    assert!(!shown.contains("has no tests/gamma.rs"), "{shown}");

    let own = project();
    rc(own.path(), &["add", "task", "Fix", "--parent", "1", "--acceptance", ACCEPTANCE, "--files", "+apps/rc/tests/gamma.rs"]);
    let shown = check_of(own.path());
    assert!(!shown.contains("has no tests/gamma.rs"), "{shown}");
}
