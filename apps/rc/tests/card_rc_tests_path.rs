//! `rc check` corrects a card's `crates/rc/tests/...` file to `apps/rc/tests/...`.

use std::path::Path;
use std::process::Command;

fn rc(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rc")).args(args).current_dir(dir).output().expect("rc runs")
}

#[test]
fn card_check_rewrites_crates_rc_tests_to_apps_rc_tests() {
    let dir = tempfile::tempdir().expect("a tempdir");
    std::fs::create_dir_all(dir.path().join("apps/rc/tests")).expect("the crate dir");
    std::fs::write(dir.path().join("apps/rc/Cargo.toml"), "[package]\nname = \"rc\"\n").expect("a manifest");
    assert!(rc(dir.path(), &["init", "--empty"]).status.success());
    assert!(rc(dir.path(), &["add", "project", "Alpha"]).status.success());
    let added = rc(dir.path(), &["add", "task", "Fix", "--parent", "1", "--files", "+crates/rc/tests/new_one.rs"]);
    assert!(added.status.success(), "{}", String::from_utf8_lossy(&added.stderr));
    rc(dir.path(), &["check"]);
    let shown = String::from_utf8_lossy(&rc(dir.path(), &["show", "1"]).stdout).into_owned();
    assert!(shown.contains("apps/rc/tests/new_one.rs"), "{shown}");
    assert!(!shown.contains("crates/rc/tests"), "{shown}");
}
