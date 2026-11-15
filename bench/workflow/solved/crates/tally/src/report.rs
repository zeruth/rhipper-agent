//! Sums of entries and the report printed from them.

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::amount::money;
use crate::parse::Entry;

/// The sum of every entry, in cents.
#[must_use]
pub fn total(entries: &[Entry]) -> i64 {
    entries.iter().map(|entry| entry.cents).sum()
}

/// The sum of each category, in the order of the category names.
#[must_use]
pub fn by_category(entries: &[Entry]) -> BTreeMap<String, i64> {
    let mut sums = BTreeMap::new();
    for entry in entries {
        *sums.entry(entry.category.clone()).or_insert(0) += entry.cents;
    }
    sums
}

/// The `n` categories with the largest sums, each with its sum: largest first, equal sums in
/// the order of their names, and every category when there are fewer than `n`.
#[must_use]
pub fn top(entries: &[Entry], n: usize) -> Vec<(String, i64)> {
    let mut sums: Vec<(String, i64)> = by_category(entries).into_iter().collect();
    sums.sort_by_key(|(_, cents)| Reverse(*cents));
    sums.truncate(n);
    sums
}

/// The report: a line for each category, then the total.
#[must_use]
pub fn render(entries: &[Entry]) -> String {
    let mut out = String::new();
    for (category, cents) in by_category(entries) {
        let _ = writeln!(out, "{category} {}", money(cents));
    }
    let _ = writeln!(out, "total {}", money(total(entries)));
    out
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
    fn the_report_sums_each_category_and_ends_with_the_total() {
        let entries = [
            entry("food", 1250),
            entry("rent", 80000),
            entry("food", 250),
        ];
        assert_eq!(render(&entries), "food 15.00\nrent 800.00\ntotal 815.00\n");
    }

    #[test]
    fn top_gives_the_largest_sums_first() {
        let entries = [entry("tea", 100), entry("rent", 900), entry("food", 400)];
        assert_eq!(
            top(&entries, 2),
            [("rent".to_owned(), 900), ("food".to_owned(), 400)]
        );
        assert_eq!(top(&entries, 5).len(), 3);
    }

    #[test]
    fn top_orders_equal_sums_by_name() {
        let entries = [entry("tea", 300), entry("food", 200), entry("food", 100)];
        assert_eq!(
            top(&entries, 2),
            [("food".to_owned(), 300), ("tea".to_owned(), 300)]
        );
    }
}
