//! The handoff's test hint names the integration tests file the card's acceptance (`--test`) or
//! its files name, and a `src/**/tests.rs` only when neither does.

use ra_context::map::{Ask, test_recipe};

fn hint(card: &str, files: &[&str]) -> String {
    let files: Vec<String> = files.iter().map(|f| (*f).to_owned()).collect();
    test_recipe(|_| None, &files, &Ask { card, ..Ask::default() })
}

#[test]
fn a_test_target_in_the_acceptance_is_the_file_the_hint_names() {
    let hint = hint("acceptance: `cargo test -p x --test snapshots`", &["crates/x/src/lib.rs"]);
    assert!(hint.contains("crates/x/tests/snapshots.rs"), "{hint}");
    assert!(!hint.contains("src/tests.rs"), "{hint}");
}

#[test]
fn a_tests_file_in_the_card_files_is_the_file_the_hint_names() {
    let hint = hint("acceptance: `cargo test -p x`", &["+crates/x/src/lib.rs", "+crates/x/tests/generate.rs"]);
    assert!(hint.contains("crates/x/tests/generate.rs"), "{hint}");
    assert!(!hint.contains("tests.rs"), "{hint}");
}

#[test]
fn a_card_naming_neither_keeps_the_src_tests_file() {
    let hint = hint("acceptance: `cargo test -p x`", &["crates/x/src/lib.rs"]);
    assert!(hint.contains("crates/x/src/tests.rs"), "{hint}");
}
