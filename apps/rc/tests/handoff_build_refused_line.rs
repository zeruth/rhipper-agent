//! The handoff of a project whose builds are refused says `cargo fmt`/`rustfmt` are the only
//! cargo commands that run; a project whose builds are allowed keeps the plain `cargo`.

use ra_gotchas::shape::block_with;

#[test]
fn a_project_whose_builds_are_refused_is_told_only_cargo_fmt_runs() {
    let block = block_with(true, true);
    assert!(block.contains("`cargo fmt`/`rustfmt` are the only cargo commands that run"), "{block}");
    assert!(block.contains("the check does the building"), "{block}");
    assert!(!block.contains("Bash runs: cargo"), "{block}");
}

#[test]
fn a_project_whose_builds_are_allowed_keeps_the_plain_cargo_allowance() {
    let block = block_with(true, false);
    assert!(block.contains("Bash runs: cargo, rustfmt, git status"), "{block}");
    assert!(!block.contains("only cargo commands"), "{block}");
    assert_eq!(block, ra_gotchas::shape::block_for(true));
}
