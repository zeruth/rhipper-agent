//! The report's pinned metrics: what the walks are worth, read from every open project's pair
//! feed and cut into sessions (a gap of an hour between events starts a new one), so each metric
//! shows the latest session and its trend over the ones before. The person's six, 2026-11-12:
//! accepted cards per million tokens, cost per accepted card, the cache-read share, read-in over
//! total input, the rerun rate and the check and review pass rate.

use std::collections::HashMap;
use std::path::PathBuf;

/// A gap this long between two events of the feeds starts a new session.
const SESSION_GAP: u64 = 3600;

/// How many sessions a metric's trend shows.
pub(crate) const SHOWN: usize = 10;

/// What one session of work came to, summed over every project.
#[derive(Debug, Default, Clone)]
pub(crate) struct Session {
    pub landed: u32,
    /// Input and output tokens of every call: the runs and the reviews.
    pub tokens: f64,
    /// The runs' input tokens, and of them the ones read from the cache and read in.
    pub run_input: f64,
    pub cached: f64,
    pub read_in: f64,
    pub cost: f64,
    pub runs: u32,
    pub reruns: u32,
    pub reviews: u32,
    pub reviews_passed: u32,
    pub checks_failed: u32,
}

/// One pinned metric: its name, what it means, its value per session (oldest first, `None`
/// where the session had nothing to measure) and whether a higher value is better.
pub(crate) struct Metric {
    pub name: &'static str,
    pub hint: &'static str,
    pub values: Vec<Option<f64>>,
    pub higher_is_better: bool,
    pub format: fn(f64) -> String,
}

impl Metric {
    /// The latest session's value with something to measure.
    pub(crate) fn current(&self) -> Option<f64> {
        self.values.iter().rev().find_map(|v| *v)
    }

    /// The latest value against the one before it: positive when it went up.
    pub(crate) fn delta(&self) -> Option<f64> {
        let mut known = self.values.iter().rev().filter_map(|v| *v);
        let now = known.next()?;
        let before = known.next()?;
        Some(now - before)
    }

    /// Whether the latest change is for the better; `None` without a change.
    pub(crate) fn improved(&self) -> Option<bool> {
        self.delta().filter(|d| d.abs() > f64::EPSILON).map(|d| (d > 0.0) == self.higher_is_better)
    }
}

/// The sessions of the feeds of `roots`, oldest first, the last [`SHOWN`] that ran something.
pub(crate) fn sessions(roots: &[PathBuf]) -> Vec<Session> {
    let mut events: Vec<(u64, String)> = Vec::new();
    for root in roots {
        let Ok(text) = std::fs::read_to_string(root.join("var").join("pair.log")) else { continue };
        for line in text.lines() {
            if let Some((at, body)) = line.split_once('\t')
                && let Ok(at) = at.trim().parse::<u64>()
            {
                events.push((at, body.to_owned()));
            }
        }
    }
    events.sort_by_key(|(at, _)| *at);
    let mut out: Vec<Session> = Vec::new();
    let mut started: HashMap<String, u32> = HashMap::new();
    let mut last = 0u64;
    for (at, body) in events {
        if out.is_empty() || at.saturating_sub(last) > SESSION_GAP {
            out.push(Session::default());
            started.clear();
        }
        last = at;
        let session = out.last_mut().expect("a session was pushed");
        if let Some(rest) = body.strip_prefix("start #") {
            let card = rest.split_whitespace().next().unwrap_or_default().to_owned();
            let seen = started.entry(card).or_insert(0);
            session.runs += 1;
            if *seen > 0 {
                session.reruns += 1;
            }
            *seen += 1;
        } else if body.starts_with("end ") {
            let input = number_after(&body, " in ").unwrap_or(0.0);
            let output = number_after(&body, " out ").unwrap_or(0.0);
            session.tokens += input + output;
            session.run_input += input;
            if let Some(share) = percent_before(&body, "% cached") {
                session.cached += input * share / 100.0;
            }
            if let Some(k) = number_after(&body, "read-in ") {
                session.read_in += k * 1000.0;
            }
            session.cost += number_after(&body, "$").unwrap_or(0.0);
            // a run whose check failed ends sent back without a review
            if body.starts_with("end ✗") && body.contains(" needs revision on ") {
                session.checks_failed += 1;
            }
        } else if body.starts_with("review ") {
            session.tokens += number_after(&body, " in ").unwrap_or(0.0) + number_after(&body, " out ").unwrap_or(0.0);
            session.cost += number_after(&body, "$").unwrap_or(0.0);
            session.reviews += 1;
            if body.starts_with("review ✓") {
                session.reviews_passed += 1;
            }
        } else if body.starts_with("landed #") {
            session.landed += 1;
        }
    }
    let ran: Vec<Session> = out.into_iter().filter(|s| s.runs > 0).collect();
    let skip = ran.len().saturating_sub(SHOWN);
    ran.into_iter().skip(skip).collect()
}

/// The six metrics over `sessions`.
pub(crate) fn metrics(sessions: &[Session]) -> Vec<Metric> {
    let per = |f: &dyn Fn(&Session) -> Option<f64>| sessions.iter().map(f).collect::<Vec<_>>();
    let ratio = |a: f64, b: f64| (b > 0.0).then(|| a / b);
    vec![
        Metric {
            name: "ACCEPTED / M TOKENS",
            hint: "cards landed for every million tokens spent: what the tokens actually bought",
            values: per(&|s| ratio(f64::from(s.landed), s.tokens / 1e6)),
            higher_is_better: true,
            format: |v| format!("{v:.2}"),
        },
        Metric {
            name: "COST / ACCEPTED",
            hint: "dollars spent, runs and reviews, for every card landed",
            values: per(&|s| ratio(s.cost, f64::from(s.landed))),
            higher_is_better: false,
            format: |v| format!("${v:.2}"),
        },
        Metric {
            name: "CACHE READ",
            hint: "the share of the runs' input read from the cache: context reuse",
            values: per(&|s| ratio(s.cached * 100.0, s.run_input)),
            higher_is_better: true,
            format: |v| format!("{v:.0}%"),
        },
        Metric {
            name: "READ-IN / INPUT",
            hint: "the share of the runs' input spent reading the code in: discovery overhead",
            values: per(&|s| ratio(s.read_in * 100.0, s.run_input)),
            higher_is_better: false,
            format: |v| format!("{v:.0}%"),
        },
        Metric {
            name: "RERUN RATE",
            hint: "runs of a card already run in the session: work that had to be done again",
            values: per(&|s| ratio(f64::from(s.reruns) * 100.0, f64::from(s.runs))),
            higher_is_better: false,
            format: |v| format!("{v:.0}%"),
        },
        Metric {
            name: "PASS RATE",
            hint: "reviews accepted over reviews and failed checks: the quality of the work",
            values: per(&|s| ratio(f64::from(s.reviews_passed) * 100.0, f64::from(s.reviews + s.checks_failed))),
            higher_is_better: true,
            format: |v| format!("{v:.0}%"),
        },
    ]
}

/// The number right after the first `marker` in `text` (`in 78259`, `$0.3336`, `read-in 92k`).
pub(crate) fn number_after(text: &str, marker: &str) -> Option<f64> {
    let at = text.find(marker)? + marker.len();
    let digits: String = text[at..].chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    digits.trim_end_matches('.').parse().ok()
}

/// The number right before `marker` (`91` of `(91% cached`).
fn percent_before(text: &str, marker: &str) -> Option<f64> {
    let end = text.find(marker)?;
    let start = text[..end].rfind(|c: char| !c.is_ascii_digit()).map_or(0, |i| i + 1);
    text[start..end].parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_sums_its_runs_reviews_and_landings() {
        let dir = std::env::temp_dir().join(format!("rc-metrics-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("var")).unwrap();
        let feed = "100\tstart #80 A · m\n\
                    200\tend ✓ #80 A implemented on m · turns 2 · in 1000 (90% cached) out 100 · read-in 1k, 0 files whole · $0.50\n\
                    201\treview ✗ #80 A needs revision (substantive) · 5 point(s) by m · in 100 out 10 · $0.10\n\
                    210\tstart #80 A · m\n\
                    300\tend ✓ #80 A implemented on m · turns 2 · in 1000 (90% cached) out 100 · read-in 1k · $0.50\n\
                    301\treview ✓ #80 A accepted by m · in 100 out 10 · $0.10\n\
                    320\tlanded #80 A · squashed\n\
                    9000\tstart #81 B · m\n\
                    9100\tend ✗ #81 B needs revision on m · turns 1 · in 500 (50% cached) out 50 · $0.20\n";
        std::fs::write(dir.join("var").join("pair.log"), feed).unwrap();
        let sessions = sessions(&[dir.clone()]);
        assert_eq!(sessions.len(), 2);
        let first = &sessions[0];
        assert_eq!((first.runs, first.reruns, first.landed, first.reviews, first.reviews_passed), (2, 1, 1, 2, 1));
        assert!((first.cost - 1.2).abs() < 1e-9);
        assert!((first.cached - 1800.0).abs() < 1e-9);
        assert_eq!(sessions[1].checks_failed, 1);
        let metrics = metrics(&sessions);
        assert_eq!((metrics[4].format)(metrics[4].values[0].unwrap()), "50%");
        assert_eq!(metrics[1].values[1], None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
