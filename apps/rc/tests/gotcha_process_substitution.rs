//! A refused `<(...)` process substitution names its gotcha, and the gotcha says to dump once into
//! `var/` and read that file.

use ra_gotchas::{Catalogue, Conditions, Verdict};

#[test]
fn a_process_substitution_matches_its_gotcha() {
    let catalogue = Catalogue::seed();
    for command in ["diff <(git show a:f) <(git show b:f)", "grep x <(cargo test 2>&1)"] {
        let id = catalogue.matching(command).map(|g| g.id.as_str());
        assert_eq!(id, Some("process-substitution"), "{command}");
    }
}

#[test]
fn the_gotcha_says_to_write_once_under_var_and_read_it() {
    let catalogue = Catalogue::seed();
    let Verdict::Deny { id, reason } = catalogue.command("diff <(git show a:f) <(git show b:f)", Conditions::default()) else {
        panic!("the command is refused with a hint");
    };
    assert_eq!(id, "process-substitution");
    assert!(reason.contains("var/"), "{reason}");
    assert!(reason.contains("Grep or Read"), "{reason}");
}
