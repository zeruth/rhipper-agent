use tally::{ParseError, parse_line};

fn is_new(error: &ParseError) -> bool {
    !matches!(error, ParseError::Empty | ParseError::NoAmount | ParseError::BadAmount(_))
}

#[test]
fn a_third_word_is_a_new_error() {
    assert!(is_new(&parse_line("food 12.50 lunch").unwrap_err()));
    assert!(is_new(&parse_line("food 1 2 3").unwrap_err()));
}

#[test]
fn two_words_still_parse() {
    assert_eq!(parse_line("  food   12.50  ").unwrap().cents, 1250);
}
