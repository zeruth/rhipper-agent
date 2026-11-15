//! A refused `cargo test … | tail` names a gotcha that says to run it without the pipe and read
//! the excerpt the check returns, instead of logging as unmatched.

use ra_gotchas::{Catalogue, Conditions, Verdict};

const PIPED: [&str; 4] = [
    "cargo test -p ra-gotchas -q 2>&1 | tail -n 40",
    "cargo test -p rc --test x | head -n 20",
    "cargo test --workspace 2>&1 | grep FAILED",
    "  cargo test -p ra-gotchas | tail",
];

#[test]
fn a_piped_cargo_test_matches_its_gotcha() {
    let catalogue = Catalogue::seed();
    for command in PIPED {
        let id = catalogue.matching(command).map(|g| g.id.as_str());
        assert_eq!(id, Some("piped-cargo-test"), "{command}");
    }
}

#[test]
fn the_gotcha_says_to_run_it_without_the_pipe_and_read_the_excerpt() {
    let catalogue = Catalogue::seed();
    // a pipe the shape table allows runs on its build slot; one it refuses is named
    let refused = format!("{} | tee var/out.txt", PIPED[0]);
    let Verdict::Deny { id, reason } = catalogue.command(&refused, Conditions::default()) else {
        panic!("the command is refused with a hint");
    };
    assert_eq!(id, "piped-cargo-test");
    assert!(reason.contains("without the pipe"), "{reason}");
    assert!(reason.contains("excerpt"), "{reason}");
}

#[test]
fn an_unpiped_cargo_test_does_not_match_it() {
    let catalogue = Catalogue::seed();
    let id = catalogue.matching("cargo test -p ra-gotchas -q").map(|g| g.id.as_str());
    assert_ne!(id, Some("piped-cargo-test"));
}
