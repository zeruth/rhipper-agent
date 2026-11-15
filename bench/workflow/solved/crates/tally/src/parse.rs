//! One line of a ledger into an [`Entry`].

use std::fmt;

use crate::amount::cents_of;

/// One line of a ledger: a category and an amount in cents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub category: String,
    pub cents: i64,
}

/// Why a line is not an entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// The line has no words.
    Empty,
    /// The line has a category and nothing after it.
    NoAmount,
    /// The second word is not an amount; it is carried as written.
    BadAmount(String),
    /// The line has a word after its amount.
    ExtraWord,
    /// The first word has a character outside `a` to `z`, `0` to `9` and `-`; it is carried as
    /// written.
    BadCategory(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("empty line"),
            Self::NoAmount => f.write_str("no amount after the category"),
            Self::BadAmount(word) => write!(f, "not an amount: {word}"),
            Self::ExtraWord => f.write_str("more than two words"),
            Self::BadCategory(name) => write!(f, "not a category name: {name}"),
        }
    }
}

impl std::error::Error for ParseError {}

/// Parses `category amount`: `food 12.50` is twelve and a half units of `food`.
///
/// # Errors
///
/// A [`ParseError`] that says which part of the line is missing or malformed.
pub fn parse_line(line: &str) -> Result<Entry, ParseError> {
    let mut words = line.split_whitespace();
    let category = words.next().ok_or(ParseError::Empty)?;
    if !category
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(ParseError::BadCategory(category.to_owned()));
    }
    let amount = words.next().ok_or(ParseError::NoAmount)?;
    let cents = cents_of(amount).ok_or_else(|| ParseError::BadAmount(amount.to_owned()))?;
    if words.next().is_some() {
        return Err(ParseError::ExtraWord);
    }
    Ok(Entry {
        category: category.to_owned(),
        cents,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_a_category_and_an_amount() {
        let entry = parse_line("  food   12.50 ").unwrap();
        assert_eq!((entry.category.as_str(), entry.cents), ("food", 1250));
    }

    #[test]
    fn a_line_without_its_parts_says_which_is_missing() {
        assert_eq!(parse_line("   "), Err(ParseError::Empty));
        assert_eq!(parse_line("food"), Err(ParseError::NoAmount));
        assert_eq!(
            parse_line("food x"),
            Err(ParseError::BadAmount("x".to_owned()))
        );
    }

    #[test]
    fn a_third_word_is_an_error() {
        assert_eq!(parse_line("food 12.50 lunch"), Err(ParseError::ExtraWord));
        assert_eq!(parse_line("food 1 2 3"), Err(ParseError::ExtraWord));
    }

    #[test]
    fn a_category_name_outside_its_alphabet_is_an_error() {
        assert_eq!(
            parse_line("Food 1"),
            Err(ParseError::BadCategory("Food".to_owned()))
        );
        assert_eq!(
            parse_line("12.50 food"),
            Err(ParseError::BadCategory("12.50".to_owned()))
        );
        assert_eq!(parse_line("rent-2026 800").unwrap().category, "rent-2026");
    }

    #[test]
    fn an_error_reads_as_its_sentence() {
        let sentences = [
            (ParseError::Empty, "empty line"),
            (ParseError::NoAmount, "no amount after the category"),
            (ParseError::BadAmount("x".to_owned()), "not an amount: x"),
            (ParseError::ExtraWord, "more than two words"),
            (
                ParseError::BadCategory("Food".to_owned()),
                "not a category name: Food",
            ),
        ];
        for (error, sentence) in sentences {
            assert_eq!(error.to_string(), sentence);
        }
    }
}
