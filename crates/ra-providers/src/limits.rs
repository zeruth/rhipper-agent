//! The plan's rate limits, as the Claude Code harness reports them on every run in a
//! `rate_limit_event`: the windows (`five_hour`, `seven_day`), how much of each is used, and when
//! it resets. The provider keeps the latest in `var/limits.json`; the console, the window and
//! `rc usage` read it, so a person sees the week's room without a model call.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The share of the week past which a run on the plan is steered: refused, or sent to the local
/// route when one is up. `[plan] threshold` of `rhipper.toml` moves it.
pub const DEFAULT_THRESHOLD: f64 = 0.9;

/// The name of the week's window in the harness's events.
pub const WEEK: &str = "seven_day";

/// One window of the plan.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Window {
    /// The part of the window used, 0 to 1.
    pub utilization: f64,
    /// When the window resets, as seconds since the Unix epoch.
    pub resets_at: u64,
}

/// The limits as last seen.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Limits {
    /// `allowed`, or what the harness said instead.
    pub status: String,
    /// The windows by name, `five_hour` and `seven_day`.
    pub windows: BTreeMap<String, Window>,
    /// When the event came, as seconds since the Unix epoch.
    pub seen_at: u64,
}

impl Limits {
    /// Reads a `rate_limit_event` line; `None` for any other line or an unreadable one.
    pub fn from_event(line: &str) -> Option<Self> {
        let event: serde_json::Value = serde_json::from_str(line).ok()?;
        if event.get("type").and_then(serde_json::Value::as_str) != Some("rate_limit_event") {
            return None;
        }
        let info = event.get("rate_limit_info")?;
        let mut windows = BTreeMap::new();
        if let Some(unified) = info.get("unifiedWindows").and_then(serde_json::Value::as_object) {
            for (name, window) in unified {
                let (Some(utilization), Some(resets_at)) = (
                    window.get("utilization").and_then(serde_json::Value::as_f64),
                    window.get("resetsAt").and_then(serde_json::Value::as_u64),
                ) else {
                    continue;
                };
                windows.insert(name.clone(), Window { utilization, resets_at });
            }
        }
        Some(Self {
            status: info.get("status").and_then(serde_json::Value::as_str).unwrap_or("unknown").to_owned(),
            windows,
            seen_at: now(),
        })
    }

    /// Takes the windows of a newer event over the ones held, keeping a window the new event
    /// left out, since the harness reports one window at a time.
    pub fn merge(&mut self, newer: Self) {
        self.status = newer.status;
        self.seen_at = newer.seen_at;
        self.windows.extend(newer.windows);
    }

    /// Loads the limits from `path`; the default when there is none.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    /// Writes the limits to `path`.
    ///
    /// # Errors
    /// When the file cannot be written.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    /// Appends the event to the log at `path`, one JSON line each, so the week's history
    /// outlives the latest in `limits.json`.
    ///
    /// # Errors
    /// When the file cannot be written.
    pub fn append(&self, path: &Path) -> anyhow::Result<()> {
        use std::io::Write as _;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
        writeln!(file, "{}", serde_json::to_string(self)?)?;
        Ok(())
    }

    /// Whether the limits are from the last day; older ones are stale.
    pub fn fresh(&self) -> bool {
        now().saturating_sub(self.seen_at) < 24 * 3600
    }

    /// The week's window when it has not reset and is at or past `threshold`, 0 to 1: the plan
    /// is to be spared. No reading, a reset window or room is `None`.
    #[must_use]
    pub fn week_past(&self, threshold: f64) -> Option<Window> {
        self.windows.get(WEEK).copied().filter(|w| w.resets_at > now() && w.utilization >= threshold)
    }

    /// The week's window with its room and its reset, `5% left · resets in 3d 4h`; `None`
    /// without a reading.
    #[must_use]
    pub fn week_room(&self) -> Option<String> {
        self.windows.get(WEEK).map(room)
    }

    /// The windows as short lines, `week 10% · resets in 3d 4h`.
    #[must_use]
    pub fn summary(&self) -> Vec<String> {
        self.windows
            .iter()
            .map(|(name, window)| format!("{} {:.0}% · resets in {}", label(name), window.utilization * 100.0, until(window.resets_at)))
            .collect()
    }
}

/// The room left in `window` and when it resets, `5% left · resets in 3d 4h`.
#[must_use]
pub fn room(window: &Window) -> String {
    format!("{:.0}% left · resets in {}", (1.0 - window.utilization).max(0.0) * 100.0, until(window.resets_at))
}

/// The week's utilization over the last seven days, read from the log: the last reading of
/// each day, oldest first, and the rise each day brought, so a sparkline and the steepest day
/// come without a model call.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Week {
    /// The `seven_day` utilization at the end of each of the last seven days, today last;
    /// `None` for a day without a run.
    pub days: [Option<f64>; 7],
    /// How much each day added, against the last reading before it.
    pub rises: [f64; 7],
    /// Today, as days since the Unix epoch, so the days can be named.
    pub today: u64,
}

impl Week {
    /// Reads the log at `path`; an empty week when there is none.
    pub fn load(path: &Path) -> Self {
        Self::of_log(&std::fs::read_to_string(path).unwrap_or_default(), now())
    }

    /// Builds the week from the log's lines, as of `now`; a line that does not read is skipped.
    pub fn of_log(log: &str, now: u64) -> Self {
        let today = now / 86_400;
        let first = today.saturating_sub(6);
        let mut week = Self { today, ..Self::default() };
        let mut last = None;
        let readings = log
            .lines()
            .filter_map(|line| serde_json::from_str::<Limits>(line).ok())
            .filter_map(|limits| Some((limits.seen_at / 86_400, limits.windows.get("seven_day")?.utilization)));
        for (day, utilization) in readings {
            if day < first || day > today {
                last = Some(utilization);
                continue;
            }
            #[allow(clippy::cast_possible_truncation)]
            let slot = (day - first) as usize;
            // the rise of a day counts from the reading before it; the day's own first reading
            // when it is the first ever
            let before = last.unwrap_or(utilization);
            week.rises[slot] = utilization - before;
            week.days[slot] = Some(utilization);
            last = Some(utilization);
        }
        week
    }

    /// The day the week's usage climbed most and the climb, `("Tue", 0.12)`; `None` when no
    /// day rose.
    pub fn steepest(&self) -> Option<(&'static str, f64)> {
        let mut best: Option<(usize, f64)> = None;
        for (slot, rise) in self.rises.iter().copied().enumerate() {
            if rise > 0.0 && best.is_none_or(|(_, b)| rise > b) {
                best = Some((slot, rise));
            }
        }
        best.map(|(slot, rise)| (self.day_name(slot), rise))
    }

    /// The weekday name of slot `slot`, today being the last.
    pub fn day_name(&self, slot: usize) -> &'static str {
        const NAMES: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
        // the epoch began on a Thursday
        let day = self.today.saturating_sub(6) + slot as u64;
        NAMES[usize::try_from(day % 7).unwrap_or(0)]
    }
}

/// The short name of a window, for a badge: `5h`, `week`, `week+overage`.
pub fn label(name: &str) -> &str {
    match name {
        "five_hour" => "5h",
        "seven_day" => "week",
        "seven_day_overage_included" => "week+overage",
        "seven_day_opus" => "week opus",
        "seven_day_sonnet" => "week sonnet",
        other => other,
    }
}

/// How long until `at`, as `3d 4h`, `4h 12m` or `12m`; `now` when it passed.
pub fn until(at: u64) -> String {
    let left = at.saturating_sub(now());
    let (d, h, m) = (left / 86_400, left % 86_400 / 3600, left % 3600 / 60);
    match (d, h, m) {
        (0, 0, 0) => "now".to_owned(),
        (0, 0, m) => format!("{m}m"),
        (0, h, m) => format!("{h}h {m:02}m"),
        (d, h, _) => format!("{d}d {h}h"),
    }
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVENT: &str = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","resetsAt":1794513600,"rateLimitType":"seven_day","unifiedWindows":{"seven_day":{"utilization":0.1,"resetsAt":1794513600}}},"uuid":"x"}"#;

    #[test]
    fn an_event_yields_its_windows_and_other_lines_nothing() {
        let limits = Limits::from_event(EVENT).unwrap();
        assert_eq!(limits.status, "allowed");
        assert_eq!(limits.windows["seven_day"], Window { utilization: 0.1, resets_at: 1_794_513_600 });
        assert!(Limits::from_event(r#"{"type":"assistant"}"#).is_none());
        assert!(Limits::from_event("not json").is_none());
    }

    #[test]
    fn a_merge_keeps_the_window_the_newer_event_left_out() {
        let mut limits = Limits::from_event(EVENT).unwrap();
        let five = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","unifiedWindows":{"five_hour":{"utilization":0.42,"resetsAt":1794400000}}}}"#;
        limits.merge(Limits::from_event(five).unwrap());
        assert_eq!(limits.windows.len(), 2);
        assert_eq!(limits.summary()[0].split(' ').next(), Some("5h"));
        assert!(limits.summary()[1].starts_with("week 10%"), "{:?}", limits.summary());
    }

    #[test]
    fn the_file_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("var/limits.json");
        assert_eq!(Limits::load(&path), Limits::default());
        let limits = Limits::from_event(EVENT).unwrap();
        limits.save(&path).unwrap();
        assert_eq!(Limits::load(&path), limits);
        assert!(limits.fresh());
    }

    #[test]
    fn two_events_append_two_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("var/limits.log");
        let limits = Limits::from_event(EVENT).unwrap();
        limits.append(&path).unwrap();
        limits.append(&path).unwrap();
        let log = std::fs::read_to_string(&path).unwrap();
        assert_eq!(log.lines().count(), 2);
        assert_eq!(serde_json::from_str::<Limits>(log.lines().next().unwrap()).unwrap(), limits);
    }

    #[test]
    fn the_week_reads_its_days_and_names_the_steepest() {
        let now = 1_794_600_000;
        let line = |days_ago: u64, utilization: f64| {
            let mut limits = Limits::from_event(EVENT).unwrap();
            limits.seen_at = now - days_ago * 86_400;
            limits.windows.get_mut("seven_day").unwrap().utilization = utilization;
            serde_json::to_string(&limits).unwrap()
        };
        let log = [line(9, 0.5), line(3, 0.1), line(3, 0.2), line(1, 0.5), line(0, 0.55), "junk".to_owned()].join("\n");
        let week = Week::of_log(&log, now);
        assert_eq!(week.days, [None, None, None, Some(0.2), None, Some(0.5), Some(0.55)]);
        assert_eq!(week.steepest().map(|(_, rise)| (rise * 100.0).round()), Some(30.0));
        assert_eq!(week.steepest().unwrap().0, week.day_name(5));
        assert_eq!(Week::of_log("", now).steepest(), None);
    }

    #[test]
    fn the_week_is_past_the_threshold_at_it_and_not_once_reset() {
        let mut limits = Limits { seen_at: now(), ..Default::default() };
        assert_eq!(limits.week_past(DEFAULT_THRESHOLD), None);
        limits.windows.insert(WEEK.to_owned(), Window { utilization: 0.9, resets_at: now() + 7200 });
        assert!(limits.week_past(DEFAULT_THRESHOLD).is_some());
        assert_eq!(limits.week_past(0.95), None);
        assert_eq!(limits.week_room().as_deref(), Some("10% left · resets in 2h 00m"));
        limits.windows.get_mut(WEEK).unwrap().resets_at = now() - 1;
        assert_eq!(limits.week_past(DEFAULT_THRESHOLD), None);
    }

    #[test]
    fn until_reads_as_a_span() {
        assert_eq!(until(0), "now");
        assert_eq!(until(now() + 90), "1m");
        assert_eq!(until(now() + 3 * 3600 + 5 * 60 + 3), "3h 05m");
        assert_eq!(until(now() + 2 * 86_400 + 3 * 3600), "2d 3h");
    }
}
