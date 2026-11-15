//! The prompt's search guidance says a Grep over Rust sources uses `type: rust`, and that a glob
//! starting with `*/` under a path matches nothing.

use ra_gotchas::shape;

#[test]
fn the_prompt_says_to_grep_rust_with_a_type_not_a_star_slash_glob() {
    for rg in [true, false] {
        let block = shape::block_for(rg);
        assert!(block.contains("`type: rust`"), "{block}");
        assert!(block.contains("starting with `*/`"), "{block}");
    }
}
