//! The trace setup: events go to the log file always, and, when the caller passes a sink, they
//! mirror into the console's log pane, where `debug` shows or hides the low-level lines. The
//! screen itself never sees them; raw mode would scribble over the UI.

use std::sync::Mutex;

use tracing::field::{Field, Visit};
use tracing_subscriber::layer::Layer;
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;

use crate::logs::{Kind, Sink};

/// The target of the worker's progress lines: what it reads, edits and runs, as it happens.
pub const PROGRESS: &str = "ra::progress";

/// The span a walk runs each card's future in, `tracing::info_span!(CARD_SPAN, id)`: the
/// progress lines inside it are tagged with the card, so several runs at once keep their
/// steps apart in the activity file and the window.
pub const CARD_SPAN: &str = "card";

/// The card a span is for, kept on the span for the layers to read.
struct CardId(i64);

/// Reads the `id` field of a card span.
#[derive(Default)]
struct IdField(Option<i64>);

impl Visit for IdField {
    fn record_i64(&mut self, field: &Field, value: i64) {
        if field.name() == "id" {
            self.0 = Some(value);
        }
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        if field.name() == "id" {
            self.0 = i64::try_from(value).ok();
        }
    }

    fn record_debug(&mut self, _: &Field, _: &dyn std::fmt::Debug) {}
}

/// The layer that notes each card span's id, for the progress layers to tag lines with.
struct Cards;

impl<S> Layer<S> for Cards
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &tracing::span::Attributes<'_>, id: &tracing::span::Id, ctx: tracing_subscriber::layer::Context<'_, S>) {
        if attrs.metadata().name() != CARD_SPAN {
            return;
        }
        let mut field = IdField::default();
        attrs.record(&mut field);
        if let (Some(card), Some(span)) = (field.0, ctx.span(id)) {
            span.extensions_mut().insert(CardId(card));
        }
    }
}

/// The card an event happened under, from the nearest card span around it.
fn card_of_event<S>(event: &tracing::Event<'_>, ctx: &tracing_subscriber::layer::Context<'_, S>) -> Option<i64>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    ctx.event_scope(event)?.find_map(|span| span.extensions().get::<CardId>().map(|c| c.0))
}

/// A progress line tagged with its card when it has one.
fn tagged(text: String, card: Option<i64>) -> String {
    match card {
        Some(card) => crate::logs::tag_card(card, &text),
        None => text,
    }
}

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
    tracing_subscriber::registry().with(filter).with(Cards).with(file_layer).with(bridge).with(activity).init();
}

/// The layer that appends the progress lines to the activity file, one per line, each tagged
/// with its card when it ran under one.
struct Activity {
    file: Mutex<std::fs::File>,
}

impl<S> Layer<S> for Activity
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, ctx: tracing_subscriber::layer::Context<'_, S>) {
        if event.metadata().target() != PROGRESS {
            return;
        }
        let mut visitor = Text::default();
        event.record(&mut visitor);
        let line = tagged(visitor.text.replace('\n', " "), card_of_event(event, &ctx));
        if let Ok(mut file) = self.file.lock() {
            let _ = std::io::Write::write_all(&mut *file, format!("{line}\n").as_bytes());
        }
    }
}

/// The layer that mirrors events into the console's log pane.
struct Bridge {
    sink: Sink,
}

impl<S> Layer<S> for Bridge
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, ctx: tracing_subscriber::layer::Context<'_, S>) {
        let mut visitor = Text::default();
        event.record(&mut visitor);
        // a progress line reads as work, not as a trace
        if event.metadata().target() == PROGRESS {
            self.sink.progress(&tagged(visitor.text, card_of_event(event, &ctx)));
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
