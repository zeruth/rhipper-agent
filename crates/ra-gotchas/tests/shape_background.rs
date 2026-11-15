use ra_gotchas::shape::block_with;

#[test]
fn the_block_tells_a_build_to_start_in_the_background() {
    let block = block_with(true, false);
    assert!(block.contains("`run_in_background` on its first call"), "{block}");
    assert!(block.contains("`cargo build`, `cargo test` or `cargo clippy`"), "{block}");
}

#[test]
fn a_block_that_refuses_builds_does_not_ask_for_one() {
    assert!(!block_with(true, true).contains("run_in_background"));
}
