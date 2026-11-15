//! An awk field `$1` inside single quotes is not a shell variable: a chain with `;` between
//! pipelines matches `piped-or-chained`, never `shell-variable-command`.

use ra_gotchas::{Catalogue, Conditions, Verdict};

const COMMAND: &str = "grep -n x f | awk -F: '$1>6540 && $1<6620' ; grep -n y f | cut -c1-9";

#[test]
fn a_single_quoted_awk_field_does_not_match_the_shell_variable_gotcha() {
    let catalogue = Catalogue::seed();
    let matched = catalogue.matching(COMMAND).map(|g| g.id.clone());
    assert_ne!(matched.as_deref(), Some("shell-variable-command"));
}

#[test]
fn a_chain_with_an_awk_field_is_refused_as_piped_or_chained() {
    let Verdict::Deny { id, .. } = Catalogue::seed().command(COMMAND, Conditions::default()) else {
        panic!("{COMMAND} is refused with a hint");
    };
    assert_eq!(id, "piped-or-chained");
}
