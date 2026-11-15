//! A bare `cargo test -q` is the full-suite gotcha's, and no message advises a tail pipe.

use ra_gotchas::Catalogue;

#[test]
fn a_bare_quiet_cargo_test_matches_the_full_suite_gotcha() {
    let catalogue = Catalogue::seed();
    let matched = catalogue.matching("cargo test -q").map(|g| g.id.clone());
    assert_eq!(matched.as_deref(), Some("worker-full-suite"));
}

#[test]
fn no_message_advises_a_tail_pipe() {
    let catalogue = Catalogue::seed();
    for id in ["long-output", "shell-outside-allowlist"] {
        let message = &catalogue.get(id).expect("the seed holds it").message;
        assert!(!message.contains("tail -n 80"), "{id}: {message}");
        assert!(message.contains("cargo test -q -p <crate>"), "{id}: {message}");
    }
}
