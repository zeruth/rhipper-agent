//! The console pane of the workspace: the session log as bubbles, the runs as framed cards
//! with their steps, humming while they work. Hovering a clipped line shows the whole of it.

use std::f32::consts::{FRAC_PI_2, PI};
use std::time::Duration;

use egui::{Align, Color32, CornerRadius, FontId, Frame, Layout, Margin, Painter, Pos2, Rect, RichText, Stroke, Ui, Vec2, pos2};
use ra_console::logs::{Kind, Line, Run, Sink};

use crate::theme;

/// The most characters a bubble shows of a line before it clips, with the rest on hover.
const CLIP: usize = 600;
/// The most characters a step shows before it clips, with the rest on hover.
const STEP_CLIP: usize = 140;
/// The corner radius of a run card, which the light follows.
const RADIUS: f32 = 10.0;
/// The points of wheel one notch is worth; a notch over a card is two rows, as the buttons are.
const NOTCH: f32 = 50.0;
/// The segments the tail of the light is drawn in.
const SEGMENTS: u8 = 48;
/// The laps the light makes along the border in a second.
const LAPS_PER_SECOND: f32 = 0.22;

/// Draws the log into `ui`, oldest first; the caller wraps it in a scroll area that sticks to
/// the bottom. `show_debug` lets the trace lines through.
pub(crate) fn draw(ui: &mut Ui, sink: &Sink, show_debug: bool) {
    let lines: Vec<Line> = sink.with_lines(<[Line]>::to_vec);
    for (index, line) in lines.iter().enumerate() {
        if matches!(line.kind, Kind::Log(tracing::Level::DEBUG | tracing::Level::TRACE)) && !show_debug {
            continue;
        }
        let run = line.run.and_then(|id| sink.run(id).map(|run| (id, run)));
        match run {
            Some((id, run)) if line.kind == Kind::Run => run_card(ui, sink, id, &line.text, &run, index),
            _ => bubble(ui, line, index),
        }
        ui.add_space(6.0);
    }
}

/// One log entry as a rounded bubble, in the color of its kind.
fn bubble(ui: &mut Ui, line: &Line, index: usize) {
    let tint = theme::lane(1 + index % 9);
    let (border, text, fill) = match line.kind {
        Kind::Input => (theme::ACCENT, theme::TEXT, theme::PANEL_RAISED),
        Kind::Output | Kind::Diff => (tint, tint, theme::PANEL),
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
                clipped(ui, raw, CLIP, &font, color);
            }
        })
        .response;
    if line.text.chars().count() > 160 && line.text.lines().count() == 1 {
        response.on_hover_text(RichText::new(&line.text).monospace());
    }
}

/// A line of text that clips at `limit` characters, with the whole of it on hover.
fn clipped(ui: &mut Ui, raw: &str, limit: usize, font: &FontId, color: Color32) {
    let count = raw.chars().count();
    if count <= limit {
        ui.label(RichText::new(raw).font(font.clone()).color(color));
        return;
    }
    let shown: String = raw.chars().take(limit).collect();
    ui.label(RichText::new(format!("{shown}…")).font(font.clone()).color(color)).on_hover_text(RichText::new(raw).font(font.clone()));
}

/// A worker run as a card: the title and the clock or the outcome on top, the latest steps
/// inside at the run's height, a soft light flowing along the border while it is live. The
/// wheel over the card sizes the card, not the log.
fn run_card(ui: &mut Ui, sink: &Sink, id: u64, title: &str, run: &Run, index: usize) {
    let lane = theme::lane(1 + index % 9);
    let elapsed = run.started.elapsed().as_secs_f32();
    let base = if failed(run) {
        theme::ACCENT
    } else if run.live() {
        // the border breathes between the rule and the bright lane while the run works
        let pulse = ((elapsed * 1.6).sin() * 0.5 + 0.5) * 0.6 + 0.2;
        theme::mix(theme::RULE, lane, pulse)
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
            run_title(ui, sink, id, title, run, lane);
            ui.add_space(4.0);
            run_steps(ui, run);
        })
        .response;
    wheel(ui, sink, id, response.rect);
    if run.live() {
        hum(ui.painter(), response.rect, lane, elapsed);
    }
}

/// Whether the run ended badly.
fn failed(run: &Run) -> bool {
    run.outcome.as_deref().is_some_and(|o| o.starts_with('✗'))
}

/// A duration as the card's clock shows it.
fn clock(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 60 { format!("{}m{:02}s", s / 60, s % 60) } else { format!("{s}s") }
}

/// The title row of a run card: the task and the model as badges, the way the status bar wears
/// them; the clock and the step count while the run is live, the outcome and how long it took
/// when it ends; and the two buttons that size the card.
fn run_title(ui: &mut Ui, sink: &Sink, id: u64, title: &str, run: &Run, lane: Color32) {
    let status = match (&run.outcome, run.took) {
        (Some(outcome), Some(took)) => format!("{outcome} · {}", clock(took)),
        (Some(outcome), None) => outcome.clone(),
        (None, _) => format!("{} · {} steps", clock(run.started.elapsed()), run.steps.len()),
    };
    let status_color = if failed(run) {
        theme::ERROR_TEXT
    } else if run.live() {
        theme::WARN_TEXT
    } else {
        lane
    };
    ui.horizontal(|ui| {
        if let Some((task, model)) = title.rsplit_once(" · ") {
            crate::cards::badge_marked(ui, "◇", task, theme::TRACE);
            crate::cards::badge_marked(ui, "◉", model, theme::ACCENT);
        } else {
            crate::cards::badge_marked(ui, "◇", title, theme::TRACE);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            // the height of the card, two rows a notch, by the buttons or the wheel
            if ui.small_button("▾").on_hover_text("fewer rows").clicked() {
                sink.resize_run(id, -2);
            }
            if ui.small_button("▴").on_hover_text("more rows").clicked() {
                sink.resize_run(id, 2);
            }
            ui.label(RichText::new(status).color(status_color).small());
        });
    });
}

/// The body of a run card: the latest steps at the run's height. A live run keeps its height,
/// with the rows it has not filled yet left open; a finished one shrinks to its steps. The
/// steps that fell off the top fold into one line that shows them on hover.
fn run_steps(ui: &mut Ui, run: &Run) {
    let height = usize::from(run.height);
    let body = if run.live() { height } else { height.min(run.steps.len().max(1)) };
    let hidden = run.steps.len().saturating_sub(body);
    let font = FontId::monospace(12.5);
    let skip = if hidden > 0 { hidden + 1 } else { 0 };
    if hidden > 0 {
        ui.label(RichText::new(format!("⋯ {skip} earlier steps")).font(font.clone()).color(theme::DIM).italics()).on_hover_ui(|ui| {
            for step in run.steps.iter().take(skip) {
                ui.label(RichText::new(step).font(font.clone()).color(theme::TEXT));
            }
        });
    }
    if run.steps.is_empty() {
        ui.label(RichText::new(if run.live() { "starting…" } else { "no steps" }).font(font.clone()).color(theme::DIM).italics());
    }
    // a step that repeats the one before folds into it with a count at the end
    let mut shown: Vec<(&str, usize)> = Vec::new();
    for step in run.steps.iter().skip(skip) {
        match shown.last_mut() {
            Some((last, count)) if *last == step.as_str() => *count += 1,
            _ => shown.push((step.as_str(), 1)),
        }
    }
    let drawn = shown.len();
    for (step, count) in shown {
        let (tool, rest, color) = step_tool(step);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            match tool {
                Some(tool) => {
                    crate::cards::badge(ui, tool, color);
                }
                None => {
                    ui.label(RichText::new("·").color(theme::DIM).font(font.clone()));
                }
            }
            clipped(ui, rest, STEP_CLIP, &font, if tool.is_some() { theme::TEXT } else { color });
            if count > 1 {
                ui.label(RichText::new(format!("×{count}")).font(font.clone()).color(theme::WARN_TEXT).strong());
            }
        });
    }
    if run.live() {
        for _ in drawn..body {
            ui.label(RichText::new(" ").font(font.clone()));
        }
    }
}

/// The tool a step belongs to, as a badge, with the rest of the step and the color of the
/// badge: a shell command, a file tool, a search, a gotcha; a step that is the worker's own
/// words gets no badge, a dot.
fn step_tool(step: &str) -> (Option<&'static str>, &str, Color32) {
    if let Some(rest) = step.strip_prefix("$ ") {
        return (Some("sh"), rest, theme::WARN_TEXT);
    }
    if let Some(rest) = step.strip_prefix("⚠ ") {
        return (Some("gotcha"), rest, theme::ERROR_TEXT);
    }
    if let Some(rest) = step.strip_prefix("… ") {
        return (None, rest, theme::TEXT);
    }
    const TOOLS: [(&str, &str); 11] = [
        ("Read", "read"),
        ("Edit", "edit"),
        ("MultiEdit", "edit"),
        ("Write", "write"),
        ("NotebookEdit", "edit"),
        ("Grep", "grep"),
        ("Glob", "glob"),
        ("TodoWrite", "plan"),
        ("WebFetch", "fetch"),
        ("WebSearch", "search"),
        ("Task", "agent"),
    ];
    for (name, badge) in TOOLS {
        if step == name {
            return (Some(badge), "", tool_color(badge));
        }
        if let Some(rest) = step.strip_prefix(name).and_then(|r| r.strip_prefix(' ')) {
            return (Some(badge), rest, tool_color(badge));
        }
    }
    (None, step, theme::DIM)
}

/// The color of a tool's badge: reads cool, writes warm, searches in between.
fn tool_color(badge: &str) -> Color32 {
    match badge {
        "read" | "glob" => theme::DEBUG,
        "edit" | "write" => theme::ACCENT,
        "grep" | "search" | "fetch" => theme::TRACE,
        "plan" => theme::ONLINE,
        _ => theme::DIM,
    }
}

/// The wheel over a card sizes that card and nothing else: the scroll is taken before the
/// console's scroll area sees it, and every notch of it is two rows, the way the buttons go.
/// The fraction of a notch left over waits for the next, so a trackpad adds up the same way.
fn wheel(ui: &Ui, sink: &Sink, id: u64, rect: Rect) {
    if !ui.rect_contains_pointer(rect) {
        return;
    }
    let delta = ui.input_mut(|input| std::mem::replace(&mut input.smooth_scroll_delta, Vec2::ZERO).y);
    let key = ui.make_persistent_id(("run-wheel", id));
    let mut rows = 0;
    ui.data_mut(|data| {
        let rest = data.get_temp_mut_or_default::<f32>(key);
        *rest += delta;
        while *rest >= NOTCH {
            *rest -= NOTCH;
            rows += 2;
        }
        while *rest <= -NOTCH {
            *rest += NOTCH;
            rows -= 2;
        }
    });
    if rows != 0 {
        sink.resize_run(id, rows);
    }
}

/// The light that flows along a live card's border: a bright head with a soft tail, around the
/// rounded rectangle once every few seconds, a wider glow under a thin bright line.
fn hum(painter: &Painter, rect: Rect, lane: Color32, elapsed: f32) {
    let radius = RADIUS.min(rect.width() * 0.5).min(rect.height() * 0.5);
    let perimeter = perimeter_of(rect, radius);
    let head = (elapsed * LAPS_PER_SECOND).fract() * perimeter;
    let tail = 140.0f32.min(perimeter * 0.3);
    let step = tail / f32::from(SEGMENTS);
    for i in 0..SEGMENTS {
        let behind = step * f32::from(i);
        let at = head - behind;
        let near = (1.0 - behind / tail).powi(2);
        let points = [point_on(rect, radius, at), point_on(rect, radius, at - step)];
        painter.line_segment(points, Stroke::new(6.0, lane.gamma_multiply(near * 0.25)));
        painter.line_segment(points, Stroke::new(2.0, lane.gamma_multiply(near * 0.9)));
    }
}

/// The length of the border of `rect` with corners rounded by `radius`.
fn perimeter_of(rect: Rect, radius: f32) -> f32 {
    2.0 * (rect.width() + rect.height()) - 8.0 * radius + 2.0 * PI * radius
}

/// The point `distance` along the border of `rect`, with corners rounded by `radius`, clockwise
/// from where the top edge begins; any distance wraps around.
fn point_on(rect: Rect, radius: f32, distance: f32) -> Pos2 {
    let (width, height) = (rect.width() - 2.0 * radius, rect.height() - 2.0 * radius);
    let arc = FRAC_PI_2 * radius;
    let mut dist = distance.rem_euclid(perimeter_of(rect, radius));
    if dist < width {
        return pos2(rect.left() + radius + dist, rect.top());
    }
    dist -= width;
    if dist < arc {
        return corner(pos2(rect.right() - radius, rect.top() + radius), radius, dist / radius - FRAC_PI_2);
    }
    dist -= arc;
    if dist < height {
        return pos2(rect.right(), rect.top() + radius + dist);
    }
    dist -= height;
    if dist < arc {
        return corner(pos2(rect.right() - radius, rect.bottom() - radius), radius, dist / radius);
    }
    dist -= arc;
    if dist < width {
        return pos2(rect.right() - radius - dist, rect.bottom());
    }
    dist -= width;
    if dist < arc {
        return corner(pos2(rect.left() + radius, rect.bottom() - radius), radius, dist / radius + FRAC_PI_2);
    }
    dist -= arc;
    if dist < height {
        return pos2(rect.left(), rect.bottom() - radius - dist);
    }
    dist -= height;
    corner(pos2(rect.left() + radius, rect.top() + radius), radius, dist / radius + PI)
}

/// The point at `angle` on the circle of `radius` around `center`; angles turn clockwise on the
/// screen, from three o'clock.
fn corner(center: Pos2, radius: f32, angle: f32) -> Pos2 {
    center + Vec2::angled(angle) * radius
}
