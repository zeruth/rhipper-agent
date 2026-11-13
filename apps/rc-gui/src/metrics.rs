//! The report's pinned metrics: what the walks are worth. Each project's pair feed is folded into
//! its walks (a walk ends on its `walk ran` line, no gap heuristic), every run of a card numbered
//! over the whole feed, so a rerun is a rerun whichever walk it falls in. The row shows the last
//! [`SHOWN`] walks of one project or of all of them, each tile the latest walk and its trend
//! (observability, section 6): first-run pass, rerun rate, review pass, cost and wall time a
//! landed card, tokens a turn.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests;

/// How many walks a metric's trend shows.
pub(crate) const SHOWN: usize = 10;

/// What one walk of one project came to.
#[derive(Debug, Default, Clone)]
pub(crate) struct Walk {
    pub project: PathBuf,
    pub started: u64,
    pub runs: u32,
    /// Runs of a card that had run before, in this walk or an earlier one.
    pub reruns: u32,
    /// The cards whose first run ended in the walk, and of them the ones that run landed.
    pub first_runs: u32,
    pub first_run_landed: u32,
    /// The cards whose first run ended without landing.
    pub first_run_missed: Vec<i64>,
    pub landed: u32,
    /// Verdicts, each review once: a failed one is not counted again by the end line after it.
    pub reviews: u32,
    pub reviews_passed: u32,
    /// Reviews and passes by the reviewing model.
    pub reviewers: BTreeMap<String, (u32, u32)>,
    /// Dollars of the walk's runs and their reviews, and of them the runs that did not land.
    pub usd: f64,
    pub usd_not_landed: f64,
    /// Seconds of every run, from its start to its last word: landed, sent back or ended.
    pub secs: f64,
    /// Input and output tokens of every call: the runs and the reviews.
    pub tokens: f64,
    /// The runs' input tokens, and of them the ones read from the cache and read in.
    pub run_input: f64,
    pub cached: f64,
    pub read_in: f64,
    /// Each run's input tokens a turn.
    pub per_turn: Vec<f64>,
    /// Why the card of each rerun did not land on the run before: check, review, conflict …
    pub causes: BTreeMap<&'static str, u32>,
}

/// One pinned metric: its name, what it means, its value per walk (oldest first, `None` where
/// the walk had nothing to measure), what each walk's hover says and whether higher is better.
pub(crate) struct Metric {
    pub name: &'static str,
    pub hint: &'static str,
    pub values: Vec<Option<f64>>,
    pub details: Vec<String>,
    pub higher_is_better: bool,
    pub format: fn(f64) -> String,
}

impl Metric {
    /// The latest walk's value with something to measure.
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

/// One run of a card as the fold follows it.
struct Run {
    card: i64,
    walk: usize,
    nth: u32,
    started: u64,
    last: u64,
    usd: f64,
    ended: bool,
    landed: bool,
    /// Why it did not land, from the first word against it.
    cause: Option<&'static str>,
}

/// The walks of the feeds of `roots`, every project's, oldest first.
pub(crate) fn walks(roots: &[PathBuf]) -> Vec<Walk> {
    let mut all: Vec<Walk> =
        roots.iter().flat_map(|root| fold(root, &std::fs::read_to_string(root.join("var").join("pair.log")).unwrap_or_default())).collect();
    all.sort_by_key(|w| w.started);
    all
}

/// The walks of one project's `feed`, oldest first; a walk that ran nothing is not one.
pub(crate) fn fold(project: &Path, feed: &str) -> Vec<Walk> {
    let mut walks: Vec<Walk> = Vec::new();
    // whether the last walk is still going: its `walk ran` line has not come
    let mut open = false;
    let mut runs: Vec<Run> = Vec::new();
    let mut latest: HashMap<i64, usize> = HashMap::new();
    for line in feed.lines() {
        let Some((at, body)) = line.split_once('\t') else { continue };
        let Ok(at) = at.trim().parse::<u64>() else { continue };
        if body.starts_with("walk ran ") {
            open = false;
            continue;
        }
        if let Some(rest) = body.strip_prefix("start #") {
            let Some(card) = card_id(rest) else { continue };
            if !open {
                walks.push(Walk { project: project.to_owned(), started: at, ..Walk::default() });
                open = true;
            }
            let walk = walks.len() - 1;
            let before = latest.get(&card).map(|&i| &runs[i]);
            let into = &mut walks[walk];
            into.runs += 1;
            if let Some(before) = before {
                into.reruns += 1;
                *into.causes.entry(before.cause.unwrap_or("other")).or_default() += 1;
            }
            let nth = before.map_or(1, |r| r.nth + 1);
            latest.insert(card, runs.len());
            runs.push(Run { card, walk, nth, started: at, last: at, usd: 0.0, ended: false, landed: false, cause: None });
            continue;
        }
        let said_of_card = body.starts_with("end ")
            || body.starts_with("review ")
            || body.starts_with("landed #")
            || (body.starts_with('#') && (body.contains(" not reviewed: ") || body.contains(" not landed: ")));
        if !said_of_card {
            continue;
        }
        let Some(card) = body.find('#').and_then(|i| card_id(&body[i + 1..])) else { continue };
        let Some(&i) = latest.get(&card) else { continue };
        let run = &mut runs[i];
        run.last = at;
        let walk = &mut walks[run.walk];
        if body.starts_with("end ") {
            let input = field(body, "in ").unwrap_or(0.0);
            walk.tokens += input + output_of(body).unwrap_or(0.0);
            walk.run_input += input;
            walk.cached += input * cached_of(body).unwrap_or(0.0) / 100.0;
            walk.read_in += read_in_of(body).unwrap_or(0.0);
            if let Some(turns) = field(body, "turns ").filter(|t| *t > 0.0) {
                walk.per_turn.push(input / turns);
            }
            let usd = usd_of(body).unwrap_or(0.0);
            run.usd += usd;
            walk.usd += usd;
            run.ended = true;
            if body.starts_with("end ✗") {
                let head = body.split(" · ").next().unwrap_or_default();
                run.cause.get_or_insert(if head.contains(" needs revision on ") { "check" } else { "error" });
            }
        } else if let Some(rest) = body.strip_prefix("review ") {
            walk.tokens += field(body, "in ").unwrap_or(0.0) + output_of(body).unwrap_or(0.0);
            let usd = usd_of(body).unwrap_or(0.0);
            run.usd += usd;
            walk.usd += usd;
            if rest.starts_with('#') {
                run.cause.get_or_insert("no verdict");
                continue;
            }
            let passed = rest.starts_with('✓');
            walk.reviews += 1;
            walk.reviews_passed += u32::from(passed);
            let by = reviewer_of(body).unwrap_or("?").to_owned();
            let tally = walk.reviewers.entry(by).or_default();
            tally.0 += 1;
            tally.1 += u32::from(passed);
            if !passed {
                run.cause.get_or_insert("review");
            }
        } else if body.starts_with("landed #") {
            if !run.landed {
                run.landed = true;
                walk.landed += 1;
            }
        } else {
            run.cause.get_or_insert(if body.contains("merged tree fails") { "merged check" } else { "conflict" });
        }
    }
    settle(&mut walks, &runs);
    walks
}

/// Folds each run's time, its dollars not landed and whether its first run landed into the walk
/// it ran in, once the feed has said everything of it.
fn settle(walks: &mut [Walk], runs: &[Run]) {
    for run in runs {
        let walk = &mut walks[run.walk];
        walk.secs += f64::from(u32::try_from(run.last.saturating_sub(run.started)).unwrap_or(u32::MAX));
        if !run.landed {
            walk.usd_not_landed += run.usd;
        }
        if run.nth == 1 && run.ended {
            walk.first_runs += 1;
            if run.landed {
                walk.first_run_landed += 1;
            } else {
                walk.first_run_missed.push(run.card);
            }
        }
    }
}

/// The six metrics over the last [`SHOWN`] of `walks` in `scope`: one project's, or every
/// project's when `None`.
pub(crate) fn metrics(walks: &[Walk], scope: Option<&Path>) -> Vec<Metric> {
    let scoped: Vec<&Walk> = walks.iter().filter(|w| scope.is_none_or(|p| w.project == p)).collect();
    let shown = &scoped[scoped.len().saturating_sub(SHOWN)..];
    let per = |f: &dyn Fn(&Walk) -> Option<f64>| shown.iter().copied().map(f).collect::<Vec<_>>();
    let says = |f: &dyn Fn(&Walk) -> String| shown.iter().copied().map(f).collect::<Vec<_>>();
    let ratio = |a: f64, b: f64| (b > 0.0).then(|| a / b);
    let percent = |a: u32, b: u32| ratio(f64::from(a) * 100.0, f64::from(b));
    vec![
        Metric {
            name: "FIRST-RUN PASS",
            hint: "cards whose first run landed over cards whose first run ended: where the money goes",
            values: per(&|w| percent(w.first_run_landed, w.first_runs)),
            details: says(&|w| {
                if w.first_run_missed.is_empty() {
                    "every first run that ended landed".to_owned()
                } else {
                    let ids: Vec<String> = w.first_run_missed.iter().map(|id| format!("#{id}")).collect();
                    format!("not landed on their first run: {}", ids.join(" "))
                }
            }),
            higher_is_better: true,
            format: |v| format!("{v:.0}%"),
        },
        Metric {
            name: "RERUN RATE",
            hint: "runs of a card that had run before: work that had to be done again",
            values: per(&|w| percent(w.reruns, w.runs)),
            details: says(&|w| {
                if w.causes.is_empty() {
                    "no rerun".to_owned()
                } else {
                    let causes: Vec<String> = w.causes.iter().map(|(cause, n)| format!("{cause} {n}")).collect();
                    format!("reruns after: {}", causes.join(" · "))
                }
            }),
            higher_is_better: false,
            format: |v| format!("{v:.0}%"),
        },
        Metric {
            name: "REVIEW PASS",
            hint: "reviews accepted over reviews, each review counted once",
            values: per(&|w| percent(w.reviews_passed, w.reviews)),
            details: says(&|w| {
                let by: Vec<String> = w.reviewers.iter().map(|(model, (n, ok))| format!("{model} {ok} of {n}")).collect();
                if by.is_empty() { "no review".to_owned() } else { format!("by {}", by.join(" · ")) }
            }),
            higher_is_better: true,
            format: |v| format!("{v:.0}%"),
        },
        Metric {
            name: "COST / LANDED",
            hint: "dollars spent, runs and reviews, for every card landed",
            values: per(&|w| ratio(w.usd, f64::from(w.landed))),
            details: says(&|w| {
                let lost = ratio(w.usd_not_landed * 100.0, w.usd).map_or_else(|| "—".to_owned(), |v| format!("{v:.0}%"));
                let bought = ratio(f64::from(w.landed), w.tokens / 1e6).map_or_else(|| "—".to_owned(), |v| format!("{v:.2}"));
                format!("${:.2} · not landed {lost} · {bought} landed a million tokens", w.usd)
            }),
            higher_is_better: false,
            format: |v| format!("${v:.2}"),
        },
        Metric {
            name: "TOKENS / TURN",
            hint: "the median run's input tokens a turn: the context each turn carries",
            values: per(&|w| quantile(&w.per_turn, 1, 2)),
            details: says(&|w| {
                let p90 = quantile(&w.per_turn, 9, 10).map_or_else(|| "—".to_owned(), kilo);
                let share = |part: f64| ratio(part * 100.0, w.run_input).map_or_else(|| "—".to_owned(), |v| format!("{v:.0}%"));
                format!("p90 {p90} · cache read {} · read-in {} of the input", share(w.cached), share(w.read_in))
            }),
            higher_is_better: false,
            format: kilo,
        },
        Metric {
            name: "WALL / LANDED",
            hint: "the minutes of every run of the walk, start to its last word, for every card landed",
            values: per(&|w| ratio(w.secs, f64::from(w.landed))),
            details: says(&|w| format!("{} run(s) · {} in all · {} landed", w.runs, clock(w.secs), w.landed)),
            higher_is_better: false,
            format: clock,
        },
    ]
}

/// The `num`/`den` quantile of `values`, the lower of the two middle ones for an even median.
fn quantile(values: &[f64], num: usize, den: usize) -> Option<f64> {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted.get(sorted.len().checked_sub(1)? * num / den).copied()
}

/// Tokens in thousands, `78k`.
fn kilo(v: f64) -> String {
    format!("{:.0}k", v / 1000.0)
}

/// Seconds as hours and minutes, `1h 05m`, or minutes alone under the hour.
fn clock(secs: f64) -> String {
    let minutes = (secs / 60.0).round();
    let hours = (minutes / 60.0).floor();
    if hours > 0.0 { format!("{hours:.0}h {:02.0}m", minutes - hours * 60.0) } else { format!("{minutes:.0}m") }
}

/// The card id `text` starts with.
fn card_id(text: &str) -> Option<i64> {
    let end = text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len());
    text[..end].parse().ok()
}

/// The parts of a feed line after the sentence that holds the title, so a title never reads
/// as a field (`in` of "the row in the report").
fn parts(body: &str) -> impl Iterator<Item = &str> {
    body.split(" · ").skip(1).map(str::trim)
}

/// The number the part of `body` labelled `label` starts with: `turns 12, …`, `in 1000 (…`.
fn field(body: &str, label: &str) -> Option<f64> {
    parts(body).find_map(|part| part.strip_prefix(label).and_then(lead))
}

/// The output tokens of the `in … out …` part.
fn output_of(body: &str) -> Option<f64> {
    parts(body).filter(|part| part.starts_with("in ")).find_map(|part| part.split_once(" out ").and_then(|(_, out)| lead(out)))
}

/// The cached share of the input, `91` of `in 1000 (91% cached)`.
fn cached_of(body: &str) -> Option<f64> {
    let part = parts(body).find(|part| part.starts_with("in ") && part.contains("% cached"))?;
    lead(&part[part.find('(')? + 1..])
}

/// The tokens read in, `740k` as 740000.
fn read_in_of(body: &str) -> Option<f64> {
    let rest = parts(body).find_map(|part| part.strip_prefix("read-in "))?;
    let n = lead(rest)?;
    let digits = rest.find(|c: char| !c.is_ascii_digit() && c != '.').unwrap_or(rest.len());
    Some(if rest[digits..].starts_with('k') { n * 1000.0 } else { n })
}

/// The dollars of a feed line, `$0.33` or `$0.33 est.`.
pub(crate) fn usd_of(body: &str) -> Option<f64> {
    parts(body).find_map(|part| part.strip_prefix('$').and_then(lead))
}

/// The reviewing model of a review line: the word after its last `by`.
fn reviewer_of(body: &str) -> Option<&str> {
    let part = body.rsplit(" · ").find(|part| part.contains(" by "))?;
    part.rsplit_once(" by ")?.1.split_whitespace().next()
}

/// The number `text` starts with.
fn lead(text: &str) -> Option<f64> {
    let end = text.find(|c: char| !c.is_ascii_digit() && c != '.').unwrap_or(text.len());
    text[..end].trim_end_matches('.').parse().ok()
}
