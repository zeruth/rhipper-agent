use std::path::Path;
use std::process::Command;

use super::*;

/// Runs `cargo <args>` in `dir`, offline, and returns its stdout and stderr when it succeeds.
fn cargo(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("cargo").args(args).current_dir(dir).env("CARGO_NET_OFFLINE", "true").output().expect("cargo runs");
    let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "cargo {args:?} failed:\n{text}");
    text
}

#[test]
fn the_project_is_in_the_bundle() {
    let bundle = Bundle::embedded();
    for path in ["project/Cargo.toml", "project/crates/tally/src/amount.rs", "project/rhipper.toml"] {
        assert!(bundle.get(path).is_some(), "{path} is embedded");
    }
}

#[test]
#[ignore = "runs cargo on a copy of the project"]
fn project_is_clean() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    write(&Bundle::embedded(), dir.path()).expect("the project is written");
    let root = dir.path();

    cargo(root, &["test", "--workspace", "--offline", "-q"]);
    let clippy = cargo(root, &["clippy", "--workspace", "--all-targets", "--offline", "--", "-D", "warnings"]);
    assert!(!clippy.contains("warning"), "clippy prints none:\n{clippy}");
    cargo(root, &["fmt", "--all", "--check"]);

    // The latent fault the cards repair: minus 150 cents prints with two minus signs.
    std::fs::write(root.join("fault.ledger"), "food -2.50\n").expect("a ledger");
    let report = cargo(root, &["run", "-q", "--offline", "-p", "tally-cli", "--", "fault.ledger"]);
    assert!(report.contains("total -1.-50"), "the fault survives:\n{report}");
}
