//! The trace setup: events go to the log file always, and, when the caller passes a sink, they
//! mirror into the console's log pane, where `debug` shows or hides the low-level lines. The
//! screen itself never sees them; raw mode would scribble over the UI.

use std::sync::Mutex;

use tracing::field::{Field, Visit};
use tracing_subscriber::layer::Layer;
use tracing_subscriber::prelude::*;

use crate::logs::{Kind, Sink};

/// Initializes the global subscriber: the file always, the sink when given.
pub fn init(file: std::fs::File, sink: Option<Sink>) {
    let filter = tracing_subscriber::EnvFilter::from_default_env();
    let file_layer = tracing_subscriber::fmt::layer().with_ansi(false).with_writer(Mutex::new(file));
    match sink {
        Some(sink) => tracing_subscriber::registry().with(filter).with(file_layer).with(Bridge { sink }).init(),
        None => tracing_subscriber::registry().with(filter).with(file_layer).init(),
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
        self.sink.push(Kind::Log(*event.metadata().level()), visitor.text);
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
