use ra_gotchas::{Catalogue, Where};

#[test]
fn rg_missing_says_to_use_grep_or_the_grep_tool() {
    let catalogue = Catalogue::seed();
    let hits = catalogue.scan(Where::Result, "bash: rg: command not found");
    let hit = hits.iter().find(|h| h.id == "rg-missing").expect("fires");
    assert!(hit.message.contains("Grep tool"));
    assert!(hit.message.contains("`grep`"));
    assert!(catalogue.scan(Where::Result, "3 matches in src/rg.rs").is_empty());
}
