//! A card's own check timeout: a `timeout: <seconds>` line in its objective or acceptance.

use std::time::Duration;

/// The most a card may ask for a check, in seconds.
pub(super) const CEILING: u64 = 7200;

/// The seconds a `timeout:` line of `text` names, if any.
fn named(text: &str) -> Option<&str> {
    text.lines().find_map(|line| line.trim().strip_prefix("timeout:")).map(str::trim)
}

/// The timeout a card's text gives, in place of the default; `None` when it names none or a
/// value that is not a whole number of seconds from 1 to [`CEILING`].
#[must_use]
pub(super) fn card_check_timeout(texts: &[Option<&str>]) -> Option<Duration> {
    let seconds = texts.iter().flatten().find_map(|text| named(text))?.trim_end_matches('s').trim().parse::<u64>().ok()?;
    (1..=CEILING).contains(&seconds).then(|| Duration::from_secs(seconds))
}

/// Refuses a `timeout:` line in `text` that is not seconds from 1 to [`CEILING`].
///
/// # Errors
/// When the line names a value that is over the ceiling or not a number of seconds.
pub fn refuse_timeout(text: &str) -> anyhow::Result<()> {
    let Some(value) = named(text) else { return Ok(()) };
    match value.trim_end_matches('s').trim().parse::<u64>() {
        Ok(seconds) if (1..=CEILING).contains(&seconds) => Ok(()),
        Ok(seconds) => anyhow::bail!("timeout: {seconds} s is over the ceiling of {CEILING} s: not filed"),
        Err(_) => anyhow::bail!("timeout: `{value}` is not a number of seconds: not filed"),
    }
}

#[cfg(test)]
mod tests;
