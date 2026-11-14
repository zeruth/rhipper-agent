//! The worker context states one command per call and no longer says a chain passes.

use ra_gotchas::shape;

#[test]
fn the_block_says_one_command_per_call_and_not_that_a_chain_passes() {
    for block in [shape::block_for(true), shape::block_for(false)] {
        assert!(block.contains("one command per call"), "{block}");
        assert!(block.contains("no `;`, `&&` or pipe"), "{block}");
        assert!(!block.contains("`a && b`"), "{block}");
        assert!(!block.contains("a chain `a; b`"), "{block}");
    }
}
