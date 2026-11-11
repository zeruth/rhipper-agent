//! The code pane: one file of the card, opened by a click on its name under FILES and shown
//! beside the card with line numbers and the light coloring the notes' code blocks have. It
//! follows the file on disk, so a worker's edit shows as it lands, and closes with its tab;
//! until a name is clicked there is no pane, and the card keeps the width.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Color32, FontId, Label, Layout, RichText, TextStyle, TextWrapMode, Ui};

use crate::markdown::Language;
use crate::{cards, theme};

/// How often the pane looks at the file's metadata while it is open.
const FOLLOW_EVERY: Duration = Duration::from_millis(500);

/// How long the line an outline item pointed at stays lit after the pane opens on it.
const LIT_FOR: Duration = Duration::from_secs(1);

/// A file open beside the card: its text as last read, the mtime and the size it came from,
/// and the language its extension names.
pub(crate) struct CodePane {
    path: PathBuf,
    /// The lines of the file, tabs widened; empty while the file cannot be read.
    lines: Vec<String>,
    /// Why the file could not be read, shown in place of the text.
    fault: Option<String>,
    seen: Option<(SystemTime, u64)>,
    checked: Instant,
    read_at: Option<Instant>,
    language: Language,
    /// The 1-based line the next draw scrolls to, set by a click on an outline item.
    jump: Option<usize>,
    /// The 1-based line that is lit, and since when; it fades over [`LIT_FOR`].
    lit: Option<(usize, Instant)>,
}

impl CodePane {
    /// Opens `path`, relative to the project, and reads it once.
    pub(crate) fn open(path: &Path) -> Self {
        let language = Language::of(path.extension().and_then(|e| e.to_str()));
        let mut pane = Self {
            path: path.to_path_buf(),
            lines: Vec::new(),
            fault: None,
            seen: None,
            checked: Instant::now(),
            read_at: None,
            language,
            jump: None,
            lit: None,
        };
        pane.refresh();
        pane
    }

    /// Opens `path` scrolled to `line`, 1-based, with that line lit for a moment.
    pub(crate) fn open_at(path: &Path, line: usize) -> Self {
        let mut pane = Self::open(path);
        pane.show_line(line);
        pane
    }

    /// Scrolls to `line`, 1-based, at the next draw and lights it for a moment: what a click
    /// on an outline item does to a pane already open on the file.
    pub(crate) fn show_line(&mut self, line: usize) {
        let line = line.max(1);
        self.jump = Some(line);
        self.lit = Some((line, Instant::now()));
    }

    /// How far the lit line still shows, 1 when it was just lit, 0 once it has faded.
    fn lit_fade(&self, n: usize) -> f32 {
        match self.lit {
            Some((line, since)) if line == n + 1 => (1.0 - since.elapsed().as_secs_f32() / LIT_FOR.as_secs_f32()).max(0.0),
            _ => 0.0,
        }
    }

    /// The path the pane shows, as the card names it.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Looks at the file again when the last look is old enough, and reads it when it moved.
    pub(crate) fn follow(&mut self) {
        if self.checked.elapsed() < FOLLOW_EVERY {
            return;
        }
        self.refresh();
    }

    /// Reads the file when its mtime or its size differ from the last read; a fault replaces
    /// the text, so a file a worker deleted says so instead of showing stale lines.
    pub(crate) fn refresh(&mut self) {
        self.checked = Instant::now();
        let stamp = std::fs::metadata(&self.path).ok().map(|m| (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len()));
        if stamp.is_some() && stamp == self.seen {
            return;
        }
        self.seen = stamp;
        match std::fs::read_to_string(&self.path) {
            Ok(text) => {
                self.lines = text.lines().map(|l| l.replace('\t', "    ")).collect();
                self.fault = None;
            }
            Err(e) => {
                self.lines.clear();
                self.fault = Some(format!("cannot read {}: {e}", self.path.display()));
            }
        }
        self.read_at = Some(Instant::now());
    }

    /// Draws the tab row and the text; `true` when the tab's cross was clicked.
    pub(crate) fn draw(&mut self, ui: &mut Ui) -> bool {
        let mut close = false;
        // the lit line fades frame by frame, then stops asking for frames
        match self.lit {
            Some((_, since)) if since.elapsed() >= LIT_FOR => self.lit = None,
            Some(_) => ui.ctx().request_repaint_after(Duration::from_millis(40)),
            None => {}
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let name = self.path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
            cards::tab(ui, name, theme::debug(), true);
            if ui.small_button(RichText::new("×").color(theme::dim())).on_hover_text("close the pane").clicked() {
                close = true;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let when = self.read_at.map_or_else(String::new, |at| {
                    let secs = at.elapsed().as_secs();
                    if secs < 5 { "read just now".to_owned() } else { format!("read {secs}s ago") }
                });
                ui.label(
                    RichText::new(format!("{} lines · follows the file on disk · {when}", self.lines.len())).color(theme::dim()).small(),
                );
            });
        });
        ui.label(RichText::new(self.path.display().to_string()).color(theme::dim()).small().monospace());
        ui.add_space(4.0);
        if let Some(fault) = &self.fault {
            ui.label(RichText::new(fault).color(theme::error_text()));
            return close;
        }
        // only the rows in view are laid out: a file of thousands of lines scrolls as a short
        // one does, and the gutter is as wide as the last line number needs
        let font = ui.style().text_styles.get(&TextStyle::Monospace).cloned().unwrap_or_else(|| FontId::monospace(13.0));
        let height = ui.text_style_height(&TextStyle::Monospace);
        let digits = self.lines.len().max(1).to_string().len();
        let mut area = egui::ScrollArea::both().id_salt(("code", &self.path)).auto_shrink([false, false]);
        // a jump puts the line a third of the way down, so what it opens is in view under it
        if let Some(line) = self.jump.take() {
            let row = height + ui.spacing().item_spacing.y;
            area = area.vertical_scroll_offset((row_offset(line, row) - ui.available_height() * 0.3).max(0.0));
        }
        area.show_rows(ui, height, self.lines.len(), |ui, rows| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for n in rows {
                ui.add(Label::new(self.row(n, digits, &font)).wrap_mode(TextWrapMode::Extend).selectable(true));
            }
        });
        close
    }

    /// One line as it is drawn: the number in the gutter, dim, then the colored pieces; the
    /// line a click pointed at carries a background that fades.
    fn row(&self, n: usize, digits: usize, font: &FontId) -> LayoutJob {
        let fade = self.lit_fade(n);
        let background = if fade > 0.0 { theme::debug().gamma_multiply(0.35 * fade) } else { Color32::TRANSPARENT };
        let format = |color| TextFormat { background, ..TextFormat::simple(font.clone(), color) };
        let mut job = LayoutJob::default();
        job.append(&format!("{:>digits$}  ", n + 1), 0.0, format(cards::alpha(theme::dim(), 170)));
        for (text, color) in self.language.tokens(&self.lines[n]) {
            job.append(text, 0.0, format(color));
        }
        job
    }
}

/// The scroll offset of a 1-based line when every row is `row` tall.
#[allow(clippy::cast_precision_loss)]
fn row_offset(line: usize, row: f32) -> f32 {
    line.saturating_sub(1) as f32 * row
}

#[cfg(test)]
mod tests;
