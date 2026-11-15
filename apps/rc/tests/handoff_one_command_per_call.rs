//! The worker handoff's shell section says a chained command is refused whole.

use ra_gotchas::shape;

#[test]
fn the_handoff_says_a_chained_command_is_refused_whole() {
    let line = "a command chained with `;` or `&&`, or one that tests `$?`, is refused whole: send each command in its own call";
    for block in [shape::block_for(true), shape::block_for(false)] {
        assert!(block.contains(line), "{block}");
    }
}
