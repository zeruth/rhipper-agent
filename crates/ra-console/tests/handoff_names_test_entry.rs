use ra_context::map::{Ask, test_recipe};

fn recipe(manifest: &str, file: &str) -> String {
    let read = |path: &str| (path == "crates/x/Cargo.toml").then(|| manifest.to_owned());
    let files = vec![file.to_owned()];
    let ask = Ask { card: "Objective: x", points: &[], read: &[], sources: &[] };
    test_recipe(read, &files, &ask)
}

#[test]
fn a_crate_with_autotests_off_gets_the_entry_to_add() {
    let out = recipe("[package]\nname = \"x\"\nautotests = false\n", "+crates/x/tests/new_one.rs");
    assert!(out.contains("crates/x/Cargo.toml"), "{out}");
    assert!(out.contains("[[test]]"), "{out}");
    assert!(out.contains("name = \"new_one\"") && out.contains("path = \"tests/new_one.rs\""), "{out}");
}

#[test]
fn a_crate_with_listed_test_targets_gets_the_entry_to_add() {
    let out = recipe("[package]\nname = \"x\"\n\n[[test]]\nname = \"a\"\npath = \"tests/a.rs\"\n", "+crates/x/tests/b.rs");
    assert!(out.contains("path = \"tests/b.rs\""), "{out}");
}

#[test]
fn a_crate_with_automatic_discovery_gets_no_line() {
    let out = recipe("[package]\nname = \"x\"\n", "+crates/x/tests/new_one.rs");
    assert!(!out.contains("[[test]]"), "{out}");
}

#[test]
fn an_entry_already_listed_adds_no_line() {
    let out = recipe("[package]\nautotests = false\n\n[[test]]\nname = \"b\"\npath = \"tests/b.rs\"\n", "+crates/x/tests/b.rs");
    assert!(!out.contains("[[test]]"), "{out}");
}
