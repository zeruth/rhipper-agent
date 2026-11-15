//! Amounts as text and as cents.

/// The cents of `12`, `12.5` or `12.50`: whole units and at most two decimals.
pub(crate) fn cents_of(text: &str) -> Option<i64> {
    let (units, decimals) = text.split_once('.').unwrap_or((text, "0"));
    if decimals.is_empty() || decimals.len() > 2 || !decimals.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let units: i64 = units.parse().ok()?;
    let mut fraction: i64 = decimals.parse().ok()?;
    if decimals.len() == 1 {
        fraction *= 10;
    }
    Some(units * 100 + fraction)
}

/// Cents as units with two decimals: `1250` is `12.50`.
#[must_use]
pub fn money(cents: i64) -> String {
    format!("{}.{:02}", cents / 100, cents % 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_amount_is_units_and_up_to_two_decimals() {
        assert_eq!(cents_of("12.50"), Some(1250));
        assert_eq!(cents_of("800"), Some(80000));
        assert_eq!(cents_of("0.5"), Some(50));
        assert_eq!(cents_of("1.234"), None);
        assert_eq!(cents_of("1."), None);
    }

    #[test]
    fn cents_print_with_two_decimals() {
        assert_eq!(money(1250), "12.50");
        assert_eq!(money(5), "0.05");
    }
}
