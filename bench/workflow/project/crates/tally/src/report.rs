//! Sums of entries and the report printed from them.

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
}
