//! One line of a ledger into an [`Entry`].

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
}

/// Parses `category amount`: `food 12.50` is twelve and a half units of `food`.
///
/// # Errors
///
/// A [`ParseError`] that says which part of the line is missing or malformed.
pub fn parse_line(line: &str) -> Result<Entry, ParseError> {
    let mut words = line.split_whitespace();
    let category = words.next().ok_or(ParseError::Empty)?;
    let amount = words.next().ok_or(ParseError::NoAmount)?;
    let cents = cents_of(amount).ok_or_else(|| ParseError::BadAmount(amount.to_owned()))?;
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
}
