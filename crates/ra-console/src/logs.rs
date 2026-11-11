//! The session log: an in-memory ring of lines that the UI renders and `tracing` feeds. The same
//! sink serves plain logging when no terminal is attached.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;

/// What kind of line this is; it picks the prefix and the color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A command the user typed.
    Input,
    /// An answer of the console or a worker.
    Output,
    /// The greeting of the console.
    Banner,
    /// A change report whose lines carry their own meaning: `+` names an addition, `-` a removal,
    /// and any other line reads as commentary. The bubble colors the lines like a diff.
    Diff,
    /// A logged event, at a level.
    Log(tracing::Level),
    /// A worker run: one bubble that holds the run's steps, its live state and its outcome. The
    /// line's text is the run's title; the rest lives in [`Run`], found by the line's `run` id.
    Run,
    /// A failure.
    Error,
    /// A long answer folded under its first line: the text's first line is the header, the rest
    /// the body, shown only while the entry is unfolded. A context preview comes this way.
    Folded,
}

/// One line of the log.
#[derive(Debug, Clone)]
pub struct Line {
    pub kind: Kind,
    pub text: String,
    /// The run a [`Kind::Run`] line draws.
    pub run: Option<u64>,
    /// Whether a [`Kind::Folded`] entry shows only its header.
    pub folded: bool,
}

/// A progress line that opens a run: the rest of the line is its title.
pub const RUN_START: &str = "\u{25c6}start ";
/// A progress line that closes the open run: the rest of the line is its outcome.
pub const RUN_END: &str = "\u{25c6}end ";

/// The rows a run bubble shows of its steps, unless the wheel changed it.
pub const RUN_HEIGHT: u16 = 6;
/// The fewest and the most rows the wheel gives a run bubble.
pub const RUN_HEIGHT_RANGE: (u16, u16) = (2, 30);
/// How long a finished run stays open before it folds to its header by itself.
pub const FOLD_AFTER: std::time::Duration = std::time::Duration::from_mins(1);

/// One worker run as the log draws it.
#[derive(Debug, Clone)]
pub struct Run {
    /// The steps of the work, oldest first.
    pub steps: Vec<String>,
    pub started: std::time::Instant,
    /// How long the run took, once it finished.
    pub took: Option<std::time::Duration>,
    /// The outcome, `✓ $0.12` or `✗ reason`, once the run finished.
    pub outcome: Option<String>,
    /// The rows of steps the bubble shows.
    pub height: u16,
    /// Whether the person folded the card to its header, or unfolded it; `None` leaves it to
    /// the clock, which folds a finished run after [`FOLD_AFTER`].
    pub folded: Option<bool>,
}

impl Run {
    /// Whether the run is still going.
    pub fn live(&self) -> bool {
        self.outcome.is_none()
    }

    /// Whether the card shows only its header: the person's choice when there is one, else a
    /// finished run a minute after its end. A live run never folds by itself.
    pub fn folded(&self) -> bool {
        if self.live() {
            return self.folded.unwrap_or(false);
        }
        self.folded.unwrap_or_else(|| self.took.is_some_and(|took| self.started.elapsed().saturating_sub(took) >= FOLD_AFTER))
    }
}

/// The sink the UI and the logging drain share.
#[derive(Clone)]
pub struct Sink {
    inner: Arc<Inner>,
}

struct Inner {
    lines: Mutex<Vec<Line>>,
    /// The runs, by id, and the id the next one takes.
    runs: Mutex<(std::collections::HashMap<u64, Run>, u64)>,
    /// Bumped on every push, so the UI knows when to redraw.
    version: AtomicU64,
    passthrough: Mutex<bool>,
}

impl Sink {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                lines: Mutex::new(Vec::new()),
                runs: Mutex::new((std::collections::HashMap::new(), 1)),
                version: AtomicU64::new(0),
                passthrough: Mutex::new(false),
            }),
        }
    }

    /// Takes one progress line: a start marker opens a run, an end marker closes the newest
    /// live one, and any other line is a step of the newest live run, which a step without one
    /// opens untitled.
    pub fn progress(&self, text: &str) {
        if let Some(title) = text.strip_prefix(RUN_START) {
            self.open_run(title);
        } else if let Some(outcome) = text.strip_prefix(RUN_END) {
            if let Some(id) = self.live_run() {
                self.close_run(id, outcome);
            }
        } else {
            let id = self.live_run().unwrap_or_else(|| self.open_run("worker"));
            if let Some(run) = self.inner.runs.lock().0.get_mut(&id) {
                run.steps.push(text.to_owned());
            }
            self.bump();
        }
    }

    /// Opens a run with its title and returns its id.
    pub fn open_run(&self, title: &str) -> u64 {
        let id = {
            let mut runs = self.inner.runs.lock();
            let id = runs.1;
            runs.1 += 1;
            let run =
                Run { steps: Vec::new(), started: std::time::Instant::now(), took: None, outcome: None, height: RUN_HEIGHT, folded: None };
            runs.0.insert(id, run);
            id
        };
        self.push_line(Line { kind: Kind::Run, text: title.to_owned(), run: Some(id), folded: false });
        id
    }

    /// Closes a run with its outcome. The run's line moves to the end of the log: a live card
    /// holds the bottom while it works, and it is released where it is when it ends, after
    /// whatever arrived meanwhile, not back where it began.
    pub fn close_run(&self, id: u64, outcome: &str) {
        if let Some(run) = self.inner.runs.lock().0.get_mut(&id) {
            run.took = Some(run.started.elapsed());
            run.outcome = Some(outcome.to_owned());
        }
        let mut lines = self.inner.lines.lock();
        if let Some(at) = lines.iter().position(|l| l.kind == Kind::Run && l.run == Some(id))
            && at + 1 < lines.len()
        {
            let line = lines.remove(at);
            lines.push(line);
        }
        drop(lines);
        self.bump();
    }

    /// The newest run that is still going.
    pub fn live_run(&self) -> Option<u64> {
        self.inner.runs.lock().0.iter().filter(|(_, r)| r.live()).map(|(id, _)| *id).max()
    }

    /// A copy of one run, for the draw.
    pub fn run(&self, id: u64) -> Option<Run> {
        self.inner.runs.lock().0.get(&id).cloned()
    }

    /// Grows or shrinks a run bubble by `rows`, within [`RUN_HEIGHT_RANGE`].
    pub fn resize_run(&self, id: u64, rows: i32) {
        if let Some(run) = self.inner.runs.lock().0.get_mut(&id) {
            let (low, high) = RUN_HEIGHT_RANGE;
            run.height = u16::try_from((i32::from(run.height) + rows).clamp(i32::from(low), i32::from(high))).unwrap_or(RUN_HEIGHT);
        }
        self.bump();
    }

    /// Folds a run card to its header, or unfolds it, against what it shows now; the choice
    /// outlasts the clock's.
    pub fn toggle_fold(&self, id: u64) {
        if let Some(run) = self.inner.runs.lock().0.get_mut(&id) {
            run.folded = Some(!run.folded());
        }
        self.bump();
    }

    fn bump(&self) {
        self.inner.version.fetch_add(1, Ordering::Relaxed);
    }

    /// Appends one line and wakes the UI.
    pub fn push(&self, kind: Kind, text: impl Into<String>) {
        self.push_line(Line { kind, text: text.into(), run: None, folded: false });
    }

    /// Appends `lines` folded under their first one, which reads as the header until
    /// [`Sink::toggle_newest_fold`] opens the body.
    pub fn push_folded(&self, lines: &[String]) {
        self.push_line(Line { kind: Kind::Folded, text: lines.join("\n"), run: None, folded: true });
    }

    /// Unfolds the newest folded entry, or folds it again; whether it is folded now, `None`
    /// when the log holds no such entry.
    pub fn toggle_newest_fold(&self) -> Option<bool> {
        let folded = {
            let mut lines = self.inner.lines.lock();
            let line = lines.iter_mut().rev().find(|l| l.kind == Kind::Folded)?;
            line.folded = !line.folded;
            line.folded
        };
        self.bump();
        Some(folded)
    }

    fn push_line(&self, line: Line) {
        let text = line.text.clone();
        let mut lines = self.inner.lines.lock();
        lines.push(line);
        if lines.len() > 10_000 {
            let drop = lines.len() - 10_000;
            lines.drain(..drop);
        }
        drop(lines);
        self.inner.version.fetch_add(1, Ordering::Relaxed);
        if *self.inner.passthrough.lock() {
            eprintln!("{text}");
        }
    }

    /// The version, bumped on every push.
    pub fn version(&self) -> u64 {
        self.inner.version.load(Ordering::Relaxed)
    }

    /// Runs `f` over the lines; the UI calls it once per draw.
    pub fn with_lines<R>(&self, f: impl FnOnce(&[Line]) -> R) -> R {
        let lines = self.inner.lines.lock();
        f(&lines)
    }

    /// Empties the view.
    pub fn clear(&self) {
        self.inner.lines.lock().clear();
        self.inner.version.fetch_add(1, Ordering::Relaxed);
    }

    /// After this, every push also goes to stderr, for runs without the TUI.
    pub fn set_passthrough(&self, on: bool) {
        *self.inner.passthrough.lock() = on;
    }
}

impl Default for Sink {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_land_in_the_open_run_and_a_closed_run_keeps_its_outcome() {
        let sink = Sink::new();
        sink.progress(&format!("{RUN_START}#15 Files · opus-5-5"));
        sink.progress("Read crates/store.rs");
        sink.progress("$ cargo test");
        let first = sink.live_run().unwrap();
        sink.progress(&format!("{RUN_END}✓ $0.12"));
        assert_eq!(sink.live_run(), None);
        let run = sink.run(first).unwrap();
        assert_eq!(run.steps, ["Read crates/store.rs", "$ cargo test"]);
        assert_eq!(run.outcome.as_deref(), Some("✓ $0.12"));
        assert!(run.took.is_some());
        // the bubble is one line of the log, titled
        sink.with_lines(|lines| {
            assert_eq!(lines.len(), 1);
            assert_eq!(lines[0].kind, Kind::Run);
            assert_eq!(lines[0].text, "#15 Files · opus-5-5");
        });
        // a run that ends after other lines arrived is released after them, not where it began
        sink.progress(&format!("{RUN_START}#16 Next · opus-5-5"));
        sink.push(Kind::Output, "a line that came while it worked".to_owned());
        sink.push(Kind::Output, "and another".to_owned());
        sink.progress(&format!("{RUN_END}✓"));
        sink.with_lines(|lines| {
            let kinds: Vec<Kind> = lines.iter().map(|l| l.kind).collect();
            assert_eq!(kinds, [Kind::Run, Kind::Output, Kind::Output, Kind::Run]);
            assert_eq!(lines[3].text, "#16 Next · opus-5-5");
        });
    }

    #[test]
    fn a_step_without_a_run_opens_one_and_the_height_clamps() {
        let sink = Sink::new();
        sink.progress("Grep fn insert");
        let id = sink.live_run().unwrap();
        assert_eq!(sink.run(id).unwrap().steps, ["Grep fn insert"]);
        sink.resize_run(id, -100);
        assert_eq!(sink.run(id).unwrap().height, RUN_HEIGHT_RANGE.0);
        sink.resize_run(id, 100);
        assert_eq!(sink.run(id).unwrap().height, RUN_HEIGHT_RANGE.1);
        // an end with no live run is ignored
        sink.progress(&format!("{RUN_END}✓"));
        sink.progress(&format!("{RUN_END}✓"));
        assert!(sink.run(id).unwrap().outcome.is_some());
    }

    #[test]
    fn a_folded_entry_opens_on_toggle_and_the_newest_one_answers() {
        let sink = Sink::new();
        assert_eq!(sink.toggle_newest_fold(), None);
        sink.push_folded(&["context #30".to_owned(), "the body".to_owned()]);
        sink.push(Kind::Output, "a plain answer after it");
        sink.with_lines(|lines| {
            assert_eq!(lines[0].kind, Kind::Folded);
            assert!(lines[0].folded);
            assert_eq!(lines[0].text, "context #30\nthe body");
        });
        assert_eq!(sink.toggle_newest_fold(), Some(false));
        assert_eq!(sink.toggle_newest_fold(), Some(true));
    }

    #[test]
    fn a_run_folds_on_toggle_and_a_live_one_never_by_itself() {
        let sink = Sink::new();
        sink.progress("Read a.rs");
        let id = sink.live_run().unwrap();
        // a live run stays open until the person folds it, and a toggle unfolds it again
        assert!(!sink.run(id).unwrap().folded());
        sink.toggle_fold(id);
        assert!(sink.run(id).unwrap().folded());
        sink.toggle_fold(id);
        assert!(!sink.run(id).unwrap().folded());
        // a run that just finished is open; one that finished a while ago has folded itself,
        // once no choice of the person stands in the way
        sink.progress(&format!("{RUN_END}✓"));
        let mut run = sink.run(id).unwrap();
        run.folded = None;
        assert!(!run.folded());
        run.started -= FOLD_AFTER * 2;
        assert!(run.folded());
        // the person's choice outlasts the clock
        run.folded = Some(false);
        assert!(!run.folded());
    }
}
