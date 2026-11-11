//! The console pane of the workspace: the session log as bubbles, the runs as framed cards
//! with their steps, humming while they work. Hovering a clipped line shows the whole of it.

use egui::{Align, Color32, CornerRadius, FontId, Frame, Layout, Margin, RichText, Stroke, StrokeKind, Ui};
use ra_console::logs::{Kind, Line, Run, Sink};

use crate::theme;

/// The most characters a bubble shows of a line before it clips, with the rest on hover.
const CLIP: usize = 600;

/// Draws the log into `ui`, oldest first; the caller wraps it in a scroll area that sticks to
/// the bottom. `now` is the frame time in seconds, for the hum of a live run. `show_debug` lets
/// the trace lines through.
pub(crate) fn draw(ui: &mut Ui, sink: &Sink, now: f64, show_debug: bool) {
    let lines: Vec<Line> = sink.with_lines(<[Line]>::to_vec);
    let total = lines.len();
    for (index, line) in lines.iter().enumerate() {
        if matches!(line.kind, Kind::Log(tracing::Level::DEBUG) | Kind::Log(tracing::Level::TRACE)) && !show_debug {
            continue;
        }
        match (line.kind, line.run.and_then(|id| sink.run(id))) {
            (Kind::Run, Some(run)) => run_card(ui, sink, line.run.unwrap_or(0), &line.text, &run, index, now),
            _ => bubble(ui, line, index, total),
        }
        ui.add_space(6.0);
    }
}

/// One log entry as a rounded bubble, in the color of its kind.
fn bubble(ui: &mut Ui, line: &Line, index: usize, _total: usize) {
    let lane = theme::lane(1 + index % 9);
    let (border, text, fill) = match line.kind {
        Kind::Input => (theme::ACCENT, theme::TEXT, theme::PANEL_RAISED),
        Kind::Output | Kind::Diff => (lane, lane, theme::PANEL),
        Kind::Banner => (theme::ACCENT, theme::ACCENT, theme::PANEL),
        Kind::Error | Kind::Log(tracing::Level::ERROR) => (theme::ACCENT, theme::ERROR_TEXT, theme::PANEL),
        Kind::Log(tracing::Level::WARN) => (theme::WARN, theme::WARN_TEXT, theme::PANEL),
        Kind::Log(tracing::Level::DEBUG) => (theme::DEBUG, theme::DEBUG, theme::PANEL),
        Kind::Log(tracing::Level::TRACE) => (theme::TRACE, theme::TRACE, theme::PANEL),
        Kind::Log(_) | Kind::Run => (theme::DIM, theme::TEXT, theme::PANEL),
    };
    let frame = Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 6));
    let response = frame
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let font = FontId::monospace(13.0);
            if line.kind == Kind::Input {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("❯").font(font.clone()).color(theme::ACCENT).strong());
                    ui.label(RichText::new(&line.text).font(font).color(text));
                });
                return;
            }
            for (i, raw) in line.text.lines().enumerate() {
                if i >= 40 {
                    ui.label(
                        RichText::new(format!("… {} more lines", line.text.lines().count() - 40))
                            .font(font.clone())
                            .color(theme::DIM)
                            .italics(),
                    );
                    break;
                }
                let color = match (line.kind, raw.chars().next()) {
                    (Kind::Diff, Some('+')) => theme::ONLINE,
                    (Kind::Diff, Some('-')) => theme::ERROR_TEXT,
                    _ => text,
                };
                let shown: String = raw.chars().take(CLIP).collect();
                let label = ui.label(
                    RichText::new(if raw.chars().count() > CLIP { format!("{shown}…") } else { shown }).font(font.clone()).color(color),
                );
                if raw.chars().count() > CLIP {
                    label.on_hover_text(RichText::new(raw).font(font.clone()));
                }
            }
        })
        .response;
    if line.text.chars().count() > 160 && line.text.lines().count() == 1 {
        response.on_hover_text(RichText::new(&line.text).monospace());
    }
}

/// A worker run as a card: the title and the clock or the outcome on top, the latest steps
/// inside at the run's height, a soft light flowing along the border while it is live.
fn run_card(ui: &mut Ui, sink: &Sink, id: u64, title: &str, run: &Run, index: usize, now: f64) {
    let lane = theme::lane(1 + index % 9);
    let failed = run.outcome.as_deref().is_some_and(|o| o.starts_with('✗'));
    let base = if failed {
        theme::ACCENT
    } else if run.live() {
        // the border breathes between the dim and the bright lane while the run works
        let pulse = ((now * 1.6).sin() as f32 * 0.5 + 0.5) * 0.6 + 0.2;
        theme::mix(theme::RULE, lane, pulse)
    } else {
        lane
    };
    let clock = |d: std::time::Duration| {
        let s = d.as_secs();
        if s >= 60 { format!("{}m{:02}s", s / 60, s % 60) } else { format!("{s}s") }
    };
    let status = match (&run.outcome, run.took) {
        (Some(outcome), Some(took)) => format!("{outcome} · {}", clock(took)),
        (Some(outcome), None) => outcome.clone(),
        (None, _) => format!("{} · {} steps", clock(run.started.elapsed()), run.steps.len()),
    };
    let status_color = if failed {
        theme::ERROR_TEXT
    } else if run.live() {
        theme::WARN_TEXT
    } else {
        lane
    };
    let frame = Frame::new()
        .fill(theme::PANEL)
        .stroke(Stroke::new(1.5, base))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(10, 8));
    let response = frame
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).strong().color(theme::TEXT));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    // the height of the card, two rows a notch, by the buttons or the wheel
                    if ui.small_button("▾").on_hover_text("fewer rows").clicked() {
                        sink.resize_run(id, -2);
                    }
                    if ui.small_button("▴").on_hover_text("more rows").clicked() {
                        sink.resize_run(id, 2);
                    }
                    ui.label(RichText::new(&status).color(status_color).small());
                });
            });
            ui.add_space(4.0);
            let height = usize::from(run.height);
            let body = if run.live() { height } else { height.min(run.steps.len().max(1)) };
            let hidden = run.steps.len().saturating_sub(body);
            let font = FontId::monospace(12.5);
            if hidden > 0 {
                let folded = run.steps.len() - body + 1;
                let label = ui.label(RichText::new(format!("⋯ {folded} earlier steps")).font(font.clone()).color(theme::DIM).italics());
                label.on_hover_ui(|ui| {
                    for step in run.steps.iter().take(folded) {
                        ui.label(RichText::new(step).font(font.clone()).color(theme::TEXT));
                    }
                });
            }
            let skip = if hidden > 0 { hidden + 1 } else { 0 };
            if run.steps.is_empty() {
                ui.label(RichText::new(if run.live() { "starting…" } else { "no steps" }).font(font.clone()).color(theme::DIM).italics());
            }
            for step in run.steps.iter().skip(skip) {
                let color = if step.starts_with("$ ") {
                    theme::WARN_TEXT
                } else if step.starts_with("⚠ ") {
                    theme::ERROR_TEXT
                } else if step.starts_with("… ") {
                    theme::TEXT
                } else {
                    theme::DIM
                };
                ui.horizontal(|ui| {
                    ui.label(RichText::new("▸").color(if run.live() { theme::ACCENT } else { theme::DIM }).font(font.clone()));
                    let shown: String = step.chars().take(140).collect();
                    let label = ui.label(
                        RichText::new(if step.chars().count() > 140 { format!("{shown}…") } else { shown }).font(font.clone()).color(color),
                    );
                    if step.chars().count() > 140 {
                        label.on_hover_text(RichText::new(step).font(font.clone()));
                    }
                });
            }
            // the rows a live run has not filled yet stay open, so the card keeps its height
            if run.live() {
                for _ in run.steps.len().saturating_sub(skip)..body {
                    ui.label(RichText::new(" ").font(font.clone()));
                }
            }
        })
        .response;
    // the wheel over the card sizes the card, not the log
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() > 0.5 && ui.input(|i| i.modifiers.ctrl) {
            sink.resize_run(id, if scroll > 0.0 { 1 } else { -1 });
        }
    }
    if run.live() {
        hum(ui, response.rect, lane, now);
    }
}

/// The light that flows along a live card's border: a bright head with a soft tail, around the
/// rounded rectangle once every few seconds.
fn hum(ui: &Ui, rect: egui::Rect, lane: Color32, now: f64) {
    let painter = ui.painter();
    let perimeter = 2.0 * (rect.width() + rect.height());
    let head = ((now * 0.22) % 1.0) as f32 * perimeter;
    let tail = 140.0f32.min(perimeter * 0.3);
    let segments = 48;
    for i in 0..segments {
        let d = tail * (i as f32 / segments as f32);
        let at = (head - d).rem_euclid(perimeter);
        let alpha = (1.0 - d / tail).powi(2);
        let color = Color32::from_rgba_unmultiplied(lane.r(), lane.g(), lane.b(), (alpha * 220.0) as u8);
        let (a, b) = (point_on(rect, at), point_on(rect, (at + tail / segments as f32).min(perimeter)));
        painter.line_segment([a, b], Stroke::new(2.5, color));
    }
    // the corners stay rounded under the light
    painter.rect_stroke(rect, CornerRadius::same(10), Stroke::new(1.0, Color32::TRANSPARENT), StrokeKind::Outside);
}

/// The point `distance` along the border of `rect`, clockwise from the top-left corner.
fn point_on(rect: egui::Rect, distance: f32) -> egui::Pos2 {
    let (w, h) = (rect.width(), rect.height());
    let d = distance.rem_euclid(2.0 * (w + h));
    if d < w {
        egui::pos2(rect.left() + d, rect.top())
    } else if d < w + h {
        egui::pos2(rect.right(), rect.top() + (d - w))
    } else if d < 2.0 * w + h {
        egui::pos2(rect.right() - (d - w - h), rect.bottom())
    } else {
        egui::pos2(rect.left(), rect.bottom() - (d - 2.0 * w - h))
    }
}
