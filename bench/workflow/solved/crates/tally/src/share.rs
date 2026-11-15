//! A category's share of a ledger's total.

use crate::parse::Entry;
use crate::report::{by_category, total};

/// The sum of `category` as a whole percent of the total, halves rounded up: 0 when the total
/// is not above zero, the category is absent or its sum is negative, and never above 100.
#[must_use]
pub fn share(entries: &[Entry], category: &str) -> u8 {
    let total = total(entries);
    let sum = by_category(entries).get(category).copied().unwrap_or(0);
    if total <= 0 || sum <= 0 {
        return 0;
    }
    u8::try_from((sum * 200 + total) / (total * 2)).map_or(100, |percent| percent.min(100))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(category: &str, cents: i64) -> Entry {
        Entry {
            category: category.to_owned(),
            cents,
        }
    }

    #[test]
    fn share_rounds_halves_up() {
        let entries = [entry("a", 125), entry("b", 875)];
        assert_eq!((share(&entries, "a"), share(&entries, "b")), (13, 88));
    }

    #[test]
    fn share_is_zero_without_a_positive_total_or_sum() {
        assert_eq!(share(&[], "a"), 0);
        assert_eq!(share(&[entry("a", 100), entry("b", -100)], "a"), 0);
        assert_eq!(share(&[entry("a", 300), entry("b", -100)], "b"), 0);
    }

    #[test]
    fn share_is_never_above_one_hundred() {
        assert_eq!(share(&[entry("a", 300), entry("b", -100)], "a"), 100);
    }
}
