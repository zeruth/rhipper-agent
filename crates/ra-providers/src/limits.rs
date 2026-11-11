//! The plan's rate limits, as the Claude Code harness reports them on every run in a
//! `rate_limit_event`: the windows (`five_hour`, `seven_day`), how much of each is used, and when
//! it resets. The provider keeps the latest in `var/limits.json`; the console, the window and
//! `rc usage` read it, so a person sees the week's room without a model call.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

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

    /// Whether the limits are from the last day; older ones are stale.
    pub fn fresh(&self) -> bool {
        now().saturating_sub(self.seen_at) < 24 * 3600
    }

    /// The windows as short lines, `week 10% · resets in 3d 4h`.
    pub fn summary(&self) -> Vec<String> {
        self.windows
            .iter()
            .map(|(name, window)| {
                let label = match name.as_str() {
                    "five_hour" => "5h",
                    "seven_day" => "week",
                    other => other,
                };
                format!("{label} {:.0}% · resets in {}", window.utilization * 100.0, until(window.resets_at))
            })
            .collect()
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
    fn until_reads_as_a_span() {
        assert_eq!(until(0), "now");
        assert_eq!(until(now() + 90), "1m");
        assert_eq!(until(now() + 3 * 3600 + 5 * 60 + 3), "3h 05m");
        assert_eq!(until(now() + 2 * 86_400 + 3 * 3600), "2d 3h");
    }
}
