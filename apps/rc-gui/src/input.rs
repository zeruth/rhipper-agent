//! The command line of the workspace: the same commands as the console, laid out with their
//! parts colored, so a command reads at a glance before it runs.

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId};
use ra_console::commands::COMMANDS;
use ra_roadmap::model::TaskState;

use crate::theme;

/// The command line as a layout job: the command in the accent when it is one and in the error
/// color when it is not, the ids as numbers, the models and the states in their colors, the rest
/// as text. `font` sizes it all.
pub(crate) fn highlight(text: &str, font: &FontId) -> LayoutJob {
    let mut job = LayoutJob::default();
    let known: Vec<&str> = COMMANDS.iter().map(|c| c.name).collect();
    let mut first = true;
    let mut rest = text;
    while !rest.is_empty() {
        // the whitespace between the words keeps its place
        let word_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        if word_end == 0 {
            let space_end = rest.find(|c: char| !c.is_whitespace()).unwrap_or(rest.len());
            job.append(&rest[..space_end], 0.0, format(theme::TEXT, font));
            rest = &rest[space_end..];
            continue;
        }
        let word = &rest[..word_end];
        let bare = word.trim_start_matches(['/', '.']).to_ascii_lowercase();
        let color = if first {
            if known.iter().any(|k| k.starts_with(bare.as_str())) { theme::ACCENT } else { theme::ERROR_TEXT }
        } else if word.chars().all(|c| c.is_ascii_digit()) {
            theme::WARN_TEXT
        } else if word.contains('/') || word.starts_with("claude-") {
            theme::TRACE
        } else if word.parse::<TaskState>().is_ok() {
            theme::DEBUG
        } else if matches!(word, "none" | "off" | "clear") {
            theme::DIM
        } else {
            theme::TEXT
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
    if trimmed.is_empty() || trimmed.contains(char::is_whitespace) {
        return Vec::new();
    }
    ra_console::commands::complete(&trimmed.trim_start_matches(['/', '.']).to_ascii_lowercase())
}
