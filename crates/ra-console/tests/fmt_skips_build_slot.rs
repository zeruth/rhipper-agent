//! A format compiles nothing: `cargo fmt` and `cargo fmt --check` start at once when every
//! build slot is busy, while a build still goes under `rc hold` to wait for one.

use ra_console::agent::is_build;
use ra_gotchas::{Catalogue, Conditions, Verdict};

fn held(command: &str) -> bool {
    matches!(Catalogue::seed().command(command, Conditions::default()), Verdict::Rewrite { ref command, .. } if command.starts_with("rc hold"))
}

#[test]
fn cargo_fmt_starts_at_once_without_a_build_slot() {
    for command in ["cargo fmt", "cargo fmt --check", "cargo fmt -p ra-console", "cargo fmt --all -- --check"] {
        assert!(!held(command), "{command} waited for a slot");
        assert!(!is_build(&format!("rc hold -- {command}")), "{command} counted as a build");
    }
}

#[test]
fn cargo_build_and_test_still_wait_for_a_slot() {
    for command in ["cargo build", "cargo test -p ra-console -q"] {
        assert!(held(command), "{command} did not wait for a slot");
        assert!(is_build(&format!("rc hold -- {command}")), "{command} not counted as a build");
    }
}
