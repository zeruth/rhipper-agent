//! A `;` between two commands is a chain, as `&&` and `|` are: the sed half does not hide it.

use ra_gotchas::Catalogue;

#[test]
fn a_semicolon_chain_of_cat_and_sed_n_matches_piped_or_chained() {
    let command = "cat apps/ras/tests/score.rs; sed -n 340,720p apps/ras/src/score.rs";
    let catalogue = Catalogue::seed();
    let gotcha = catalogue.matching(command).expect("a gotcha");
    assert_eq!(gotcha.id, "piped-or-chained");
    assert!(gotcha.message.contains("offset") && gotcha.message.contains("limit"), "{}", gotcha.message);
}

#[test]
fn a_lone_sed_n_still_matches_the_sed_gotcha() {
    let catalogue = Catalogue::seed();
    assert_eq!(catalogue.matching("sed -n 1,20p src/lib.rs").map(|g| g.id.clone()).as_deref(), Some("shell-outside-allowlist"));
}
