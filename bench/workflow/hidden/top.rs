use tally::{Entry, top};

fn entry(category: &str, cents: i64) -> Entry {
    Entry { category: category.to_owned(), cents }
}

fn ledger() -> Vec<Entry> {
    vec![entry("food", 200), entry("rent", 300), entry("tea", 500), entry("food", 100), entry("refund", -900)]
}

#[test]
fn largest_first_and_equal_sums_by_name() {
    assert_eq!(top(&ledger(), 2), [("tea".to_owned(), 500), ("food".to_owned(), 300)]);
}

#[test]
fn fewer_than_n_returns_them_all() {
    let names: Vec<String> = top(&ledger(), 10).into_iter().map(|(name, _)| name).collect();
    assert_eq!(names, ["tea", "food", "rent", "refund"]);
    assert!(top(&ledger(), 0).is_empty());
}

#[test]
fn the_check_of_the_card_selects_two_tests() {
    let source = include_str!("../src/report.rs");
    let selected = source.lines().map(str::trim_start).filter(|line| {
        line.starts_with("fn ") && line.split('(').next().is_some_and(|name| name.contains("top_"))
    });
    assert!(selected.count() >= 2, "fewer than two tests match the filter top_");
}
