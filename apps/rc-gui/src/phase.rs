//! What a live run card says of its run beyond the step count (observability, section 3): the
//! share of its time each kind of work took, as a strip under the title; which run of the card
//! it is and what the one before it failed on; and a clock from the run's own start on the pair
//! feed, so a window restarted thirty times a day does not set every card back to zero.

use std::collections::HashMap;
use std::path::Path;
use std::time::SystemTime;

use egui::Color32;

use crate::theme;

#[cfg(test)]
mod tests;

/// The seconds of work with no step past which a card warns that the run has gone quiet.
pub(crate) const QUIET_AFTER: u64 = 120;

/// The kind of work a step is, which the strip shares the run's time by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    /// Reading and searching: the files, the web.
    Read,
    /// Changing files.
    Edit,
    /// A shell command: a build, a check, a listing.
    Shell,
    /// The worker's own words, or a gotcha caught.
    Talk,
}

/// The phases in the order the strip draws them.
pub(crate) const PHASES: [Phase; 4] = [Phase::Read, Phase::Edit, Phase::Shell, Phase::Talk];

impl Phase {
    /// The phase of one step, by the tool it names, as the card's jewels read it.
    pub(crate) fn of(step: &str) -> Self {
        if step.starts_with("$ ") {
            return Self::Shell;
        }
        let tool = step.split(' ').next().unwrap_or_default();
        match tool {
            "Read" | "Grep" | "Glob" | "WebFetch" | "WebSearch" => Self::Read,
            "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => Self::Edit,
            _ => Self::Talk,
        }
    }

    /// The phase's colour, the one its tool's jewel wears.
    pub(crate) fn color(self) -> Color32 {
        match self {
            Self::Read => theme::debug(),
            Self::Edit => theme::accent(),
            Self::Shell => theme::warn_text(),
            Self::Talk => theme::dim(),
        }
    }
}

/// A live run as its card reads it, every time in unix seconds.
#[derive(Debug, Clone)]
pub(crate) struct Live<'a> {
    /// When the run began: the feed's start line when there is one, else the window's own
    /// sight of it.
    pub started: u64,
    /// When the run last had a step, or began.
    pub stepped: u64,
    /// Which run of its card this is, from one.
    pub number: u32,
    /// Why the run before it did not land, for a rerun.
    pub cause: Option<String>,
    pub steps: &'a [String],
}

/// The share of the run's time each phase took, in [`PHASES`] order, the phases it never
/// entered left out; the shares sum to one. The time up to the last step is shared evenly among
/// the steps (the log keeps no time per step), and the quiet since then is the last step's.
pub(crate) fn strip(live: &Live, now: u64) -> Vec<(Phase, f32)> {
    let Some(last) = live.steps.last() else { return vec![(Phase::Talk, 1.0)] };
    #[allow(clippy::cast_precision_loss)]
    let worked = live.stepped.saturating_sub(live.started) as f32;
    #[allow(clippy::cast_precision_loss)]
    let quiet = now.saturating_sub(live.stepped.max(live.started)) as f32;
    // a run with no time on it yet still shows what its steps were
    #[allow(clippy::cast_precision_loss)]
    let each = if worked > 0.0 { worked / live.steps.len() as f32 } else { 1.0 };
    let mut weight = [0.0_f32; 4];
    for step in live.steps {
        weight[index(Phase::of(step))] += each;
    }
    weight[index(Phase::of(last))] += quiet;
    let total: f32 = weight.iter().sum();
    PHASES.iter().zip(weight).filter(|(_, w)| *w > 0.0).map(|(phase, w)| (*phase, w / total)).collect()
}

fn index(phase: Phase) -> usize {
    PHASES.iter().position(|p| *p == phase).unwrap_or(0)
}

/// The seconds the run has gone without a step, once they pass [`QUIET_AFTER`].
pub(crate) fn quiet(live: &Live, now: u64) -> Option<u64> {
    Some(now.saturating_sub(live.stepped.max(live.started))).filter(|s| *s > QUIET_AFTER)
}

/// The status of a live run: its number when it is a rerun and the cause of the run before,
/// the clock from its start, its steps, and the quiet when it has gone on too long.
pub(crate) fn status(live: &Live, now: u64) -> String {
    let mut parts = Vec::new();
    if live.number > 1 {
        parts.push(format!("run {}", live.number));
        if let Some(cause) = &live.cause {
            parts.push(format!("after ✗ {cause}"));
        }
    }
    parts.push(clock(now.saturating_sub(live.started)));
    parts.push(format!("{} steps", live.steps.len()));
    if let Some(secs) = quiet(live, now) {
        parts.push(format!("⚠ quiet {}", clock(secs)));
    }
    parts.join(" · ")
}

/// Seconds as the card's clock shows them, `10m00s` or `42s`.
pub(crate) fn clock(s: u64) -> String {
    if s >= 60 { format!("{}m{:02}s", s / 60, s % 60) } else { format!("{s}s") }
}

/// The latest run of a card on the feed: when it began, which run it is, and why the one
/// before it did not land.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Begun {
    pub started: u64,
    pub number: u32,
    pub cause: Option<String>,
}

/// The latest run of every card on a project's pair feed (`time<TAB>text` lines). A run's cause
/// is the first word against it, in the words the report's rerun metric uses.
pub(crate) fn fold(feed: &str) -> HashMap<i64, Begun> {
    let mut runs: HashMap<i64, Begun> = HashMap::new();
    // the cause of each card's latest run so far, which its next run inherits
    let mut against: HashMap<i64, &'static str> = HashMap::new();
    for line in feed.lines() {
        let Some((at, body)) = line.split_once('\t') else { continue };
        let Ok(at) = at.trim().parse::<u64>() else { continue };
        if let Some(rest) = body.strip_prefix("start #") {
            let Some(card) = card_id(rest) else { continue };
            let number = runs.get(&card).map_or(1, |r| r.number + 1);
            let cause = against.remove(&card).map(str::to_owned);
            runs.insert(card, Begun { started: at, number, cause });
            continue;
        }
        // a card that landed and came back later has no failure behind it
        if let Some(rest) = body.strip_prefix("landed #") {
            if let Some(card) = card_id(rest) {
                against.remove(&card);
            }
            continue;
        }
        let Some(cause) = cause_of(body) else { continue };
        let Some(card) = body.find('#').and_then(|i| card_id(&body[i + 1..])) else { continue };
        if runs.contains_key(&card) {
            against.entry(card).or_insert(cause);
        }
    }
    runs
}

/// Why a feed line says a card's run did not land, if it says so.
fn cause_of(body: &str) -> Option<&'static str> {
    if body.starts_with("end ✗") {
        let head = body.split(" · ").next().unwrap_or_default();
        return Some(if head.contains(" needs revision on ") { "check" } else { "error" });
    }
    if let Some(rest) = body.strip_prefix("review ") {
        return if rest.starts_with('#') { Some("no verdict") } else { (!rest.starts_with('✓')).then_some("review") };
    }
    if body.starts_with('#') && (body.contains(" not reviewed: ") || body.contains(" not landed: ")) {
        return Some(if body.contains("merged tree fails") { "merged check" } else { "conflict" });
    }
    None
}

/// The card id `text` starts with.
fn card_id(text: &str) -> Option<i64> {
    let end = text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len());
    text[..end].parse().ok()
}

/// A project's pair feed folded into its cards' latest runs, read again only when the file
/// changed.
#[derive(Default)]
pub(crate) struct Feed {
    seen: Option<(SystemTime, u64)>,
    pub runs: HashMap<i64, Begun>,
}

impl Feed {
    /// Reads the feed under `root` again when it changed since the last look.
    pub(crate) fn follow(&mut self, root: &Path) {
        let path = root.join(ra_console::pair::PAIR_FILE);
        let Ok(meta) = std::fs::metadata(&path) else { return };
        let stamp = meta.modified().ok().map(|m| (m, meta.len()));
        if stamp.is_some() && stamp == self.seen {
            return;
        }
        self.seen = stamp;
        self.runs = fold(&std::fs::read_to_string(&path).unwrap_or_default());
    }

    /// The feed's latest run of the card a run card's `title` (`#12 Title · model`) names.
    pub(crate) fn of_title(&self, title: &str) -> Option<&Begun> {
        self.runs.get(&card_id(title.strip_prefix('#')?)?)
    }
}

/// The time now in unix seconds.
pub(crate) fn unix_now() -> u64 {
    ra_console::clock::unix_now()
}
