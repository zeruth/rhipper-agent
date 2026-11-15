use tally::{Entry, share};

fn entry(category: &str, cents: i64) -> Entry {
    Entry { category: category.to_owned(), cents }
}

#[test]
fn a_share_is_a_whole_percent_with_halves_up() {
    let thirds = [entry("a", 100), entry("b", 200)];
    assert_eq!((share(&thirds, "a"), share(&thirds, "b")), (33, 67));
    let eighths = [entry("a", 125), entry("b", 875)];
    assert_eq!((share(&eighths, "a"), share(&eighths, "b")), (13, 88));
}

#[test]
fn the_edges_are_zero_or_one_hundred() {
    assert_eq!(share(&[], "a"), 0);
    assert_eq!(share(&[entry("a", 100)], "b"), 0);
    assert_eq!(share(&[entry("a", 100)], "a"), 100);
    assert_eq!(share(&[entry("a", 100), entry("b", -100)], "a"), 0);
    let refund = [entry("a", 300), entry("b", -100)];
    assert_eq!((share(&refund, "a"), share(&refund, "b")), (100, 0));
}
