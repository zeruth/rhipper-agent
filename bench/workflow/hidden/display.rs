use tally::parse_line;

fn said(line: &str) -> String {
    parse_line(line).unwrap_err().to_string()
}

#[test]
fn every_variant_reads_as_its_sentence() {
    assert_eq!(said(""), "empty line");
    assert_eq!(said("food"), "no amount after the category");
    assert_eq!(said("food 1.234"), "not an amount: 1.234");
    assert_eq!(said("food 1 lunch"), "more than two words");
    assert_eq!(said("Food 1"), "not a category name: Food");
}

#[test]
fn a_parse_error_is_an_error() {
    fn is_error<E: std::error::Error>(_: &E) {}
    is_error(&parse_line("").unwrap_err());
}
