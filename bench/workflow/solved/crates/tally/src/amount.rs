//! Amounts as text and as cents.

/// The cents of `12`, `12.5` or `12.50`: whole units and at most two decimals, behind one
/// optional leading minus.
pub(crate) fn cents_of(text: &str) -> Option<i64> {
    if let Some(rest) = text.strip_prefix('-') {
        if rest.is_empty() || rest.starts_with(['-', '+']) {
            return None;
        }
        return cents_of(rest).map(|cents| -cents);
    }
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

/// Cents as units with two decimals: `1250` is `12.50`, `-50` is `-0.50`.
#[must_use]
pub fn money(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let cents = cents.unsigned_abs();
    format!("{sign}{}.{:02}", cents / 100, cents % 100)
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

    #[test]
    fn negative_amounts_parse_to_the_negative_of_the_amount() {
        assert_eq!(cents_of("-1.50"), Some(-150));
        assert_eq!(cents_of("-0.50"), Some(-50));
        assert_eq!(cents_of("-12"), Some(-1200));
    }

    #[test]
    fn negative_amounts_print_with_one_minus() {
        assert_eq!(money(-150), "-1.50");
        assert_eq!(money(-50), "-0.50");
        assert_eq!(money(-5), "-0.05");
    }

    #[test]
    fn negative_signs_do_not_make_a_bad_amount_good() {
        for text in ["--1.50", "-", "-.5", "-+1", "-1.234"] {
            assert_eq!(cents_of(text), None, "{text}");
        }
    }
}
