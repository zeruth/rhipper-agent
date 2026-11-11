//! The history of state changes: one line per change in `var/history.log` beside the roadmap,
//! appended and never rewritten, so a card's path through its states survives the file's own
//! rewrites and a `git log` that never sees `var/`. A line reads
//! `2026-11-11T20:15:03Z #12 defined -> in_progress by cli`; a line that does not parse is
//! skipped, not an error, since a hand edit of the log is nobody's fault.

use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::model::TaskState;

/// The file of the history, beside the roadmap: `var/history.log`.
pub const FILE: &str = "var/history.log";

/// Who changed a state: the surface the command came in on, or the agent loop itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Source {
    Console,
    Window,
    #[default]
    Cli,
    Mcp,
    Run,
}

impl Source {
    /// Every source, in the order the card names them.
    pub const ALL: [Self; 5] = [Self::Console, Self::Window, Self::Cli, Self::Mcp, Self::Run];

    /// The name the log writes.
    pub fn name(self) -> &'static str {
        match self {
            Self::Console => "console",
            Self::Window => "window",
            Self::Cli => "cli",
            Self::Mcp => "mcp",
            Self::Run => "run",
        }
    }
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl std::str::FromStr for Source {
    type Err = anyhow::Error;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::ALL.into_iter().find(|s| s.name() == text).with_context(|| format!("no source {text}"))
    }
}

/// One state change: when, which card, from what to what, and by whom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    /// Seconds since the epoch.
    pub at: u64,
    pub id: i64,
    pub from: TaskState,
    pub to: TaskState,
    pub by: Source,
}

impl Change {
    /// The line the log holds for the change.
    #[must_use]
    pub fn line(&self) -> String {
        format!("{} #{} {} -> {} by {}", format_time(self.at), self.id, self.from, self.to, self.by)
    }

    /// The change of one log line; `None` for a line in another shape.
    #[must_use]
    pub fn parse(line: &str) -> Option<Self> {
        let mut words = line.split_whitespace();
        let at = parse_time(words.next()?)?;
        let id = words.next()?.strip_prefix('#')?.parse().ok()?;
        let from = words.next()?.parse().ok()?;
        (words.next()? == "->").then_some(())?;
        let to = words.next()?.parse().ok()?;
        (words.next()? == "by").then_some(())?;
        let by = words.next()?.parse().ok()?;
        words.next().is_none().then_some(Self { at, id, from, to, by })
    }
}

/// The history file beside a roadmap.
#[must_use]
pub fn path_beside(roadmap: &Path) -> PathBuf {
    roadmap.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")).join(FILE)
}

/// Appends one change to the log, making `var/` when it is new.
pub fn append(log: &Path, change: &Change) -> anyhow::Result<()> {
    use std::io::Write;
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    let mut file =
        std::fs::OpenOptions::new().create(true).append(true).open(log).with_context(|| format!("cannot open {}", log.display()))?;
    writeln!(file, "{}", change.line()).with_context(|| format!("cannot write {}", log.display()))
}

/// The changes in the log, oldest first; of one card with `id`. A log not there yet is empty.
pub fn read(log: &Path, id: Option<i64>) -> anyhow::Result<Vec<Change>> {
    let text = match std::fs::read_to_string(log) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", log.display())),
    };
    Ok(text.lines().filter_map(Change::parse).filter(|c| id.is_none_or(|id| c.id == id)).collect())
}

/// Seconds since the epoch as `2026-11-11T20:15:03Z`.
#[must_use]
pub fn format_time(secs: u64) -> String {
    let (y, m, d) = civil_from_days(i64::try_from(secs / 86_400).unwrap_or(i64::MAX));
    let rest = secs % 86_400;
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rest / 3600, rest % 3600 / 60, rest % 60)
}

/// The seconds since the epoch of a `format_time` stamp; `None` for anything else.
#[must_use]
pub fn parse_time(text: &str) -> Option<u64> {
    let text = text.strip_suffix('Z')?;
    let (date, clock) = text.split_once('T')?;
    let mut date = date.splitn(3, '-').map(str::parse::<i64>);
    let mut clock = clock.splitn(3, ':').map(str::parse::<u64>);
    let (y, m, d) = (date.next()?.ok()?, date.next()?.ok()?, date.next()?.ok()?);
    let (h, min, s) = (clock.next()?.ok()?, clock.next()?.ok()?, clock.next()?.ok()?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || h > 23 || min > 59 || s > 59 {
        return None;
    }
    let days = u64::try_from(days_from_civil(y, m, d)).ok()?;
    Some(days * 86_400 + h * 3600 + min * 60 + s)
}

/// How long a span reads: `3m`, `2h 05m`, `4d 3h`; under a minute, `now`.
#[must_use]
pub fn age(secs: u64) -> String {
    let (d, h, m) = (secs / 86_400, secs % 86_400 / 3600, secs % 3600 / 60);
    match (d, h, m) {
        (0, 0, 0) => "now".to_owned(),
        (0, 0, m) => format!("{m}m"),
        (0, h, m) => format!("{h}h {m:02}m"),
        (d, h, _) => format!("{d}d {h}h"),
    }
}

// the proleptic Gregorian calendar from days since 1970-01-01, after Howard Hinnant's
// `civil_from_days`; it keeps a date crate off the roadmap engine
fn civil_from_days(days: i64) -> (i64, u64, u64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m.unsigned_abs(), d.unsigned_abs())
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests;
