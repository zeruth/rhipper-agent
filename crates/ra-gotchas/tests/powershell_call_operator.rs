use ra_gotchas::{Catalogue, Where};

const REFUSAL: &str = "Command name is a dynamic expression which is not supported: & $exe --best b.exe";

#[test]
fn a_refused_call_through_a_variable_matches_powershell_not_allowed() {
    let hits = Catalogue::seed().scan(Where::Result, REFUSAL);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "powershell-not-allowed");
}

#[test]
fn the_gotchas_text_names_the_allowed_form() {
    let catalogue = Catalogue::seed();
    let gotcha = catalogue.scan(Where::Result, REFUSAL);
    assert!(gotcha[0].message.contains("literal path"), "{}", gotcha[0].message);
    assert!(gotcha[0].message.contains("& \"C:\\path\\tool.exe\""), "{}", gotcha[0].message);
}
