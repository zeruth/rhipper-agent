//! The command line of the workspace: the same commands as the console, laid out with their
//! parts colored, so a command reads at a glance before it runs.

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId};
use ra_console::commands::COMMANDS;
use ra_roadmap::model::{Node, NodeKind, TaskState};

use crate::theme;

/// The command line as a layout job: the command in the accent when it is one and in the error
/// color when it is not, the ids as numbers, the models and the states in their colors, the rest
/// as text. `font` sizes it all.
pub(crate) fn highlight(text: &str, font: &FontId) -> LayoutJob {
    let mut job = LayoutJob::default();
    // a line without the slash is a note for the next runs, not a command: plain text
    if !text.trim_start().starts_with(['/', '.']) {
        job.append(text, 0.0, format(theme::text(), font));
        return job;
    }
    // `theme` is the window's own command, beside the console's
    let known: Vec<&str> = COMMANDS.iter().map(|c| c.name).chain(["theme"]).collect();
    let mut first = true;
    let mut rest = text;
    while !rest.is_empty() {
        // the whitespace between the words keeps its place
        let word_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        if word_end == 0 {
            let space_end = rest.find(|c: char| !c.is_whitespace()).unwrap_or(rest.len());
            job.append(&rest[..space_end], 0.0, format(theme::text(), font));
            rest = &rest[space_end..];
            continue;
        }
        let word = &rest[..word_end];
        let bare = word.trim_start_matches(['/', '.']).to_ascii_lowercase();
        let color = if first {
            if known.iter().any(|k| k.starts_with(bare.as_str())) { theme::accent() } else { theme::error_text() }
        } else if word.chars().all(|c| c.is_ascii_digit()) {
            theme::warn_text()
        } else if word.contains('/') || word.starts_with("claude-") {
            theme::trace()
        } else if word.parse::<TaskState>().is_ok() {
            theme::debug()
        } else if matches!(word, "none" | "off" | "clear") {
            theme::dim()
        } else {
            theme::text()
        };
        job.append(word, 0.0, format(color, font));
        first = false;
        rest = &rest[word_end..];
    }
    job
}

fn format(color: Color32, font: &FontId) -> TextFormat {
    TextFormat { font_id: font.clone(), color, ..Default::default() }
}

/// The command names that start with the first word of `text`, while it is still the only word.
pub(crate) fn completions(text: &str) -> Vec<&'static str> {
    let trimmed = text.trim_start();
    // only a command completes, and a command starts with the slash
    if !trimmed.starts_with(['/', '.']) || trimmed.contains(char::is_whitespace) {
        return Vec::new();
    }
    ra_console::commands::complete(&trimmed.trim_start_matches(['/', '.']).to_ascii_lowercase())
}

/// The command palette: what it has been asked and which row the keys rest on. Ctrl+K opens
/// it over the window; it closes on Enter, Escape or a click elsewhere.
#[derive(Default)]
pub(crate) struct Palette {
    pub query: String,
    pub cursor: usize,
}

/// One row of the palette: a command, or a card of the roadmap.
pub(crate) enum Item {
    Command { name: &'static str, usage: &'static str, key: Option<&'static str> },
    Card { id: i64, title: String },
}

/// The key the window already binds a command to, shown beside it in the palette.
pub(crate) fn key_of(name: &str) -> Option<&'static str> {
    match name {
        "run" => Some("Enter"),
        "clear" => Some("Ctrl+L"),
        "debug" => Some("Ctrl+D"),
        _ => None,
    }
}

/// What the palette offers for `query`: the commands whose name contains its first word, the
/// ones starting with it first, then the cards whose title or id contains the whole of it. A
/// leading slash is accepted and ignored; an empty query lists every command.
pub(crate) fn items(query: &str, nodes: &[Node]) -> Vec<Item> {
    let query = query.trim().trim_start_matches(['/', '.']).to_ascii_lowercase();
    let first = query.split_whitespace().next().unwrap_or("");
    let mut out: Vec<Item> = Vec::new();
    let mut later: Vec<Item> = Vec::new();
    for c in COMMANDS {
        let item = || Item::Command { name: c.name, usage: c.usage, key: key_of(c.name) };
        if c.name.starts_with(first) {
            out.push(item());
        } else if c.name.contains(first) {
            later.push(item());
        }
    }
    out.append(&mut later);
    if query.is_empty() {
        return out;
    }
    // a card answers to its number as well as its title; the project is not a card to pick
    for n in nodes.iter().filter(|n| n.kind != NodeKind::Project) {
        let id = n.id.to_string();
        if id.starts_with(&query) || n.title.to_ascii_lowercase().contains(&query) {
            out.push(Item::Card { id: n.id, title: n.title.clone() });
        }
    }
    out
}
