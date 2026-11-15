use tally::{ParseError, parse_line};

fn is_new(error: &ParseError) -> bool {
    !matches!(error, ParseError::Empty | ParseError::NoAmount | ParseError::BadAmount(_))
}

#[test]
fn a_name_outside_the_alphabet_is_a_new_error_that_carries_it() {
    for name in ["Food", "12.50", "a_b", "caf\u{e9}"] {
        let error = parse_line(&format!("{name} 1")).unwrap_err();
        assert!(is_new(&error), "{name}: {error:?}");
        assert!(format!("{error:?}").contains(name), "{name} is not in {error:?}");
    }
}

#[test]
fn lowercase_digits_and_dashes_pass() {
    assert_eq!(parse_line("rent-2026 800").unwrap().category, "rent-2026");
}
