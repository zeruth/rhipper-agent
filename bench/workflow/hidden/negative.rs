use tally::{money, parse_line};

fn cents(amount: &str) -> Option<i64> {
    parse_line(&format!("x {amount}")).ok().map(|entry| entry.cents)
}

#[test]
fn a_leading_minus_negates_the_amount() {
    assert_eq!(cents("-1.50"), Some(-150));
    assert_eq!(cents("-0.50"), Some(-50));
    assert_eq!(cents("-0.05"), Some(-5));
    assert_eq!(cents("-12"), Some(-1200));
    assert_eq!(cents("12.5"), Some(1250));
}

#[test]
fn money_prints_one_sign() {
    assert_eq!(money(-150), "-1.50");
    assert_eq!(money(-50), "-0.50");
    assert_eq!(money(-5), "-0.05");
    assert_eq!(money(0), "0.00");
    assert_eq!(money(1250), "12.50");
}

#[test]
fn what_failed_before_still_fails() {
    for amount in ["--1.50", "-", "-.5", "1.-5", "1.234", "-1.234", "1e2"] {
        assert_eq!(cents(amount), None, "{amount}");
    }
}

#[test]
fn the_public_surface_is_as_it_was() {
    let source = include_str!("../src/amount.rs");
    let public: Vec<&str> = source.lines().map(str::trim_start).filter(|line| line.starts_with("pub ")).collect();
    assert_eq!(public, ["pub fn money(cents: i64) -> String {"]);
}
