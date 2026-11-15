//! A format compiles nothing: `cargo fmt` and `rustfmt` pass the hook without `rc hold`, whatever
//! the slot load, while a build still goes under it.

use ra_gotchas::{Catalogue, Conditions, Verdict};

fn held(command: &str) -> bool {
    matches!(Catalogue::seed().command(command, Conditions::default()), Verdict::Rewrite { command, .. } if command.starts_with("rc hold"))
}

#[test]
fn a_format_passes_the_hold_at_once() {
    for command in ["cargo fmt", "cargo fmt --check", "cargo fmt -p rc", "rustfmt apps/rc/src/main.rs"] {
        assert!(!held(command), "{command}");
    }
}

#[test]
fn a_build_still_waits_for_a_slot() {
    for command in ["cargo build", "cargo test -p rc"] {
        assert!(held(command), "{command}");
    }
}
