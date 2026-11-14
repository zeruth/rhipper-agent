//! The prompt's tool list names `rg` only when the run shell can find it, and points at the Grep
//! tool or `grep` when it cannot.

use ra_gotchas::shape;

fn bash_list(block: &str) -> Vec<&str> {
    let list = block.lines().find_map(|l| l.strip_prefix("- Bash runs: ")).expect("a Bash line");
    list.split(", ").collect()
}

#[test]
fn rg_is_listed_when_it_is_on_the_path() {
    let block = shape::block_for(true);
    assert!(bash_list(&block).contains(&"rg"), "{block}");
    assert!(!block.contains("not on this shell's PATH"), "{block}");
}

#[test]
fn rg_is_left_out_and_grep_named_when_it_is_not() {
    let block = shape::block_for(false);
    let list = bash_list(&block);
    assert!(!list.contains(&"rg"), "{block}");
    assert!(list.contains(&"grep"), "{block}");
    assert!(block.contains("Grep tool or `grep`"), "{block}");
}
