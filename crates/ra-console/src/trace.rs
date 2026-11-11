//! The trace setup: events go to the log file always, and, when the caller passes a sink, they
//! mirror into the console's log pane, where `debug` shows or hides the low-level lines. The
//! screen itself never sees them; raw mode would scribble over the UI.

use std::sync::Mutex;

use tracing::field::{Field, Visit};
use tracing_subscriber::layer::Layer;
use tracing_subscriber::prelude::*;

use crate::logs::{Kind, Sink};

/// The target of the worker's progress lines: what it reads, edits and runs, as it happens.
pub const PROGRESS: &str = "ra::progress";

/// Initializes the global subscriber: the file always, the sink when given, and the activity
/// file when given. A process without a console writes its progress lines to the activity file,
/// and a running console tails it, so a run started from the command line or over MCP shows its
/// work in the console too.
pub fn init(file: std::fs::File, sink: Option<Sink>, activity: Option<std::fs::File>) {
    // the progress lines pass whatever RUST_LOG says
    let filter =
        tracing_subscriber::EnvFilter::from_default_env().add_directive(format!("{PROGRESS}=info").parse().expect("a valid directive"));
    let file_layer = tracing_subscriber::fmt::layer().with_ansi(false).with_writer(Mutex::new(file));
    let bridge = sink.map(|sink| Bridge { sink });
    let activity = activity.map(|file| Activity { file: Mutex::new(file) });
    tracing_subscriber::registry().with(filter).with(file_layer).with(bridge).with(activity).init();
}

/// The layer that appends the progress lines to the activity file, one per line.
struct Activity {
    file: Mutex<std::fs::File>,
}

impl<S: tracing::Subscriber> Layer<S> for Activity {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        if event.metadata().target() != PROGRESS {
            return;
        }
        let mut visitor = Text::default();
        event.record(&mut visitor);
        if let Ok(mut file) = self.file.lock() {
            let _ = std::io::Write::write_all(&mut *file, format!("{}\n", visitor.text.replace('\n', " ")).as_bytes());
        }
    }
}

/// The layer that mirrors events into the console's log pane.
struct Bridge {
    sink: Sink,
}

impl<S: tracing::Subscriber> Layer<S> for Bridge {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let mut visitor = Text::default();
        event.record(&mut visitor);
        // a progress line reads as work, not as a trace
        if event.metadata().target() == PROGRESS {
            self.sink.push(Kind::Progress, visitor.text);
        } else {
            self.sink.push(Kind::Log(*event.metadata().level()), visitor.text);
        }
    }
}

/// Collects the fields of one event into one line.
#[derive(Default)]
struct Text {
    text: String,
}

impl Visit for Text {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.text.push_str(value);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.text.push_str(&format!("{value:?}"));
        }
    }
}
