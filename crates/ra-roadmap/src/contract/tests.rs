use super::*;

#[test]
fn contract_of_an_acceptance_splits_checks_criteria_and_owner() {
    let c = contract_of("`cargo test -p x -q` passes. The list is sorted. The owner confirms it by hand on a device.");
    assert_eq!(c.checks, vec!["cargo test -p x -q"]);
    assert_eq!(c.criteria.len(), 2);
    assert_eq!(c.owner.len(), 1);
}

#[test]
fn contract_of_renders_numbered_criteria() {
    let c = contract_of("`cargo test -q` passes. The list is sorted.");
    assert_eq!(c.render(), "C1. `cargo test -q` passes.\nC2. The list is sorted.\n");
}
