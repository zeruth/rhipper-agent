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
    /// A logged event, at a level.
    Log(tracing::Level),
    /// A failure.
    Error,
}

/// One line of the log.
#[derive(Debug, Clone)]
pub struct Line {
    pub kind: Kind,
    pub text: String,
}

/// The sink the UI and the logging drain share.
#[derive(Clone)]
pub struct Sink {
    inner: Arc<Inner>,
}

struct Inner {
    lines: Mutex<Vec<Line>>,
    /// Bumped on every push, so the UI knows when to redraw.
    version: AtomicU64,
    passthrough: Mutex<bool>,
}

impl Sink {
    pub fn new() -> Self {
        Self { inner: Arc::new(Inner { lines: Mutex::new(Vec::new()), version: AtomicU64::new(0), passthrough: Mutex::new(false) }) }
    }

    /// Appends one line and wakes the UI.
    pub fn push(&self, kind: Kind, text: impl Into<String>) {
        let text: String = text.into();
        let mut lines = self.inner.lines.lock();
        lines.push(Line { kind, text: text.clone() });
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
