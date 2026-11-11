//! The console pane of the workspace: the session log as bubbles, the runs as framed cards
//! with their steps, humming while they work. Hovering a clipped line shows the whole of it.

use std::f32::consts::{FRAC_PI_2, PI};
use std::time::Duration;

use egui::{
    Align, Color32, CornerRadius, FontId, Frame, Layout, Margin, Painter, Pos2, Rect, Response, RichText, Sense, Stroke, Ui, Vec2, pos2,
};
use ra_console::logs::{Kind, Line, Run, Sink};

use crate::theme;

/// The most characters a bubble shows of a line before it clips, with the rest on hover.
const CLIP: usize = 600;
/// The most characters a step shows before it clips, with the rest on hover.
const STEP_CLIP: usize = 140;
/// The corner radius of a run card, which the light follows.
const RADIUS: f32 = 10.0;
/// The width of a step's tool jewel: the text of every step starts after it.
const JEWEL_WIDTH: f32 = 76.0;
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
    // a live run's card keeps the bottom of the log until its result is in: whatever arrives
    // while it works goes above it, and it settles into its place when it ends
    let mut live: Vec<(usize, &Line, u64, Run)> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if matches!(line.kind, Kind::Log(tracing::Level::DEBUG | tracing::Level::TRACE)) && !show_debug {
            continue;
        }
        let run = line.run.and_then(|id| sink.run(id).map(|run| (id, run)));
        match run {
            Some((id, run)) if line.kind == Kind::Run && run.live() => {
                live.push((index, line, id, run));
                continue;
            }
            Some((id, run)) if line.kind == Kind::Run => run_card(ui, sink, id, &line.text, &run, index, None),
            _ => bubble(ui, line, index),
        }
        ui.add_space(6.0);
    }
    for (index, line, id, run) in live {
        run_card(ui, sink, id, &line.text, &run, index, None);
        ui.add_space(6.0);
    }
}

/// The rows a run card gets on the dashboard: enough to follow the work, never the page.
const DASHBOARD_ROWS: f32 = 7.0;

/// The latest run's card alone, the live one before a finished one, for the dashboard;
/// `false` when the session has run nothing yet.
pub(crate) fn draw_latest_run(ui: &mut Ui, sink: &Sink) -> bool {
    let runs: Vec<(usize, u64, String)> = sink.with_lines(|lines| {
        lines.iter().enumerate().filter(|(_, l)| l.kind == Kind::Run).filter_map(|(i, l)| l.run.map(|id| (i, id, l.text.clone()))).collect()
    });
    let pick = runs.iter().rev().find(|(_, id, _)| sink.run(*id).is_some_and(|r| r.live())).or_else(|| runs.last());
    let Some((index, id, title)) = pick else { return false };
    let Some(run) = sink.run(*id) else { return false };
    // a fixed room: in a scroll area the clip is the page, and a live card would take it all
    run_card(ui, sink, *id, title, &run, *index, Some(96.0 + 24.0 * DASHBOARD_ROWS));
    true
}

/// One log entry as a rounded bubble, in the color of its kind.
fn bubble(ui: &mut Ui, line: &Line, index: usize) {
    let tint = theme::lane(1 + index % 9);
    // a line typed without the slash is a note for the next runs, and the answer to one; both
    // sit on a red field, apart from the commands and their answers
    let note = (line.kind == Kind::Input && !line.text.trim_start().starts_with(['/', '.']))
        || (line.kind == Kind::Output && line.text.starts_with("noted;"));
    let (border, text, fill) = match line.kind {
        _ if note => (theme::accent(), theme::text(), theme::mix(theme::bg(), theme::accent(), 0.22)),
        Kind::Input => (theme::accent(), theme::text(), theme::panel_raised()),
        // an answer sits on a darker field, a breath of its lane in it, so its Markdown reads
        // apart from the input above it
        Kind::Output | Kind::Diff | Kind::Folded => (tint, tint, theme::mix(theme::bg(), tint, 0.05)),
        Kind::Banner => (theme::accent(), theme::accent(), theme::panel()),
        Kind::Error | Kind::Log(tracing::Level::ERROR) => (theme::accent(), theme::error_text(), theme::panel()),
        Kind::Log(tracing::Level::WARN) => (theme::warn(), theme::warn_text(), theme::panel()),
        Kind::Log(tracing::Level::DEBUG) => (theme::debug(), theme::debug(), theme::panel()),
        Kind::Log(tracing::Level::TRACE) => (theme::trace(), theme::trace(), theme::panel()),
        Kind::Log(_) | Kind::Run => (theme::dim(), theme::text(), theme::panel()),
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
                    if note {
                        crate::cards::badge(ui, "note", theme::accent());
                    } else {
                        ui.label(RichText::new("❯").font(font.clone()).color(theme::accent()).strong());
                    }
                    ui.label(RichText::new(&line.text).font(font).color(text));
                });
                return;
            }
            // a command's answer reads as the notes do: the Markdown of the prompt renderer,
            // one line a paragraph, the indented rows as code, the outcome lines lit
            if line.kind == Kind::Output {
                crate::markdown::render_prompt(ui, &line.text, tint);
                return;
            }
            for (i, raw) in line.text.lines().enumerate() {
                if i >= 40 {
                    ui.label(
                        RichText::new(format!("… {} more lines", line.text.lines().count() - 40))
                            .font(font.clone())
                            .color(theme::dim())
                            .italics(),
                    );
                    break;
                }
                let color = match (line.kind, raw.chars().next()) {
                    (Kind::Diff, Some('+')) => theme::online(),
                    (Kind::Diff, Some('-')) => theme::error_text(),
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
/// wheel over the card sizes the card, not the log; a click on the header folds the card to
/// that one line, and a finished card folds itself after a minute.
fn run_card(ui: &mut Ui, sink: &Sink, id: u64, title: &str, run: &Run, index: usize, room: Option<f32>) {
    let lane = theme::lane(1 + index % 9);
    let elapsed = run.started.elapsed().as_secs_f32();
    let base = if failed(run) {
        theme::accent()
    } else if run.live() {
        // the border breathes between the rule and the bright lane while the run works
        let pulse = ((elapsed * 1.6).sin() * 0.5 + 0.5) * 0.6 + 0.2;
        theme::mix(theme::rule(), lane, pulse)
    } else {
        lane
    };
    let frame = Frame::new()
        .fill(theme::panel())
        .stroke(Stroke::new(1.5, base))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(10, 8));
    let response = frame
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let folded = run.folded();
            if run_title(ui, sink, id, title, run, lane, folded).clicked() {
                sink.toggle_fold(id);
            }
            if folded {
                return;
            }
            ui.add_space(4.0);
            // a live card fills the console it is in, or the room the caller gives it; a
            // finished one shrinks to its cap
            let room = room.unwrap_or_else(|| ui.clip_rect().height());
            run_steps(ui, run, room);
        })
        .response;
    if !run.folded() {
        wheel(ui, sink, id, response.rect);
    }
    // a finished card that has not folded yet is waiting on the clock, not on a push
    if !run.live() && run.folded.is_none() {
        ui.ctx().request_repaint_after(Duration::from_secs(1));
    }
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
/// when it ends; and the two buttons that size the card, which a folded card does without.
/// The row answers a click, which folds or unfolds the card.
fn run_title(ui: &mut Ui, sink: &Sink, id: u64, title: &str, run: &Run, lane: Color32, folded: bool) -> Response {
    let status = match (&run.outcome, run.took) {
        (Some(outcome), Some(took)) => format!("{outcome} · {}", clock(took)),
        (Some(outcome), None) => outcome.clone(),
        (None, _) => format!("{} · {} steps", clock(run.started.elapsed()), run.steps.len()),
    };
    let status_color = if failed(run) {
        theme::error_text()
    } else if run.live() {
        theme::warn_text()
    } else {
        lane
    };
    let row = ui.horizontal(|ui| {
        ui.label(RichText::new(if folded { "▸" } else { "▾" }).color(theme::dim()).small());
        if let Some((task, model)) = title.rsplit_once(" · ") {
            crate::cards::badge_marked(ui, "◇", task, theme::trace());
            crate::cards::badge_marked(ui, "◉", model, theme::accent());
        } else {
            crate::cards::badge_marked(ui, "◇", title, theme::trace());
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            // the height of the card, two rows a notch, by the buttons or the wheel
            if !folded {
                if ui.small_button("▾").on_hover_text("fewer rows").clicked() {
                    sink.resize_run(id, -2);
                }
                if ui.small_button("▴").on_hover_text("more rows").clicked() {
                    sink.resize_run(id, 2);
                }
            }
            ui.label(RichText::new(status).color(status_color).small());
        });
    });
    // the row is sensed after its buttons, so a click on one of them is theirs, not the fold's
    row.response.interact(Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The body of a run card: the latest steps at the run's height. A live run keeps its height,
/// with the rows it has not filled yet left open; a finished one shrinks to its steps. The
/// steps that fell off the top fold into one line that shows them on hover.
fn run_steps(ui: &mut Ui, run: &Run, room: f32) {
    let font = FontId::monospace(12.5);
    // a step that repeats the one before folds into it with a count at the end; the rows are
    // counted after the folding, so the card shows as many entries as it has rows for
    let mut folded: Vec<(&str, usize)> = Vec::new();
    for step in &run.steps {
        match folded.last_mut() {
            Some((last, count)) if *last == step.as_str() => *count += 1,
            _ => folded.push((step.as_str(), 1)),
        }
    }
    let height = usize::from(run.height);
    // a live run takes the rows the console has room for, the cap at least; a finished one its
    // cap or fewer
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let fit = ((room - 96.0) / 24.0).floor().max(0.0) as usize;
    let body = if run.live() { height.max(fit) } else { height.min(folded.len().max(1)) };
    let hidden = folded.len().saturating_sub(body);
    // the entries that fell off the top fold into one line, which takes a row of its own
    let skip = if hidden > 0 { hidden + 1 } else { 0 };
    if hidden > 0 {
        let earlier: usize = folded.iter().take(skip).map(|(_, n)| n).sum();
        ui.label(RichText::new(format!("⋯ {earlier} earlier steps")).font(font.clone()).color(theme::dim()).italics()).on_hover_ui(|ui| {
            for (step, count) in folded.iter().take(skip) {
                let text = if *count > 1 { format!("{step}  ×{count}") } else { (*step).to_owned() };
                ui.label(RichText::new(text).font(font.clone()).color(theme::text()));
            }
        });
    }
    if run.steps.is_empty() {
        ui.label(RichText::new(if run.live() { "starting…" } else { "no steps" }).font(font.clone()).color(theme::dim()).italics());
    }
    let shown: Vec<(&str, usize)> = folded.into_iter().skip(skip).collect();
    let drawn = shown.len();
    // every step on a dark field of its own, a code line's: the tool's badge, the text, the
    // count of its repeats; the worker's own words sit on the card itself
    let field = theme::mix(theme::bg(), theme::deep(), 0.6);
    for (step, count) in shown {
        let (tool, rest, color) = step_tool(step);
        // every row the same: the dark field, a jewel of one width, the text after it; the
        // worker's own words wear the edit's red jewel with no word on it
        let frame = Frame::new().fill(field).corner_radius(CornerRadius::same(5)).inner_margin(Margin::symmetric(6, 2));
        frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                match tool {
                    Some(tool) => crate::cards::jewel(ui, tool, color, JEWEL_WIDTH),
                    None => crate::cards::jewel(ui, "", theme::accent(), JEWEL_WIDTH),
                }
                clipped(ui, rest, STEP_CLIP, &font, if tool.is_some() { theme::text() } else { color });
                if count > 1 {
                    ui.label(RichText::new(format!("×{count}")).font(font.clone()).color(theme::warn_text()).strong());
                }
            });
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
        return (Some("sh"), rest, theme::warn_text());
    }
    if let Some(rest) = step.strip_prefix("⚠ ") {
        return (Some("gotcha"), rest, theme::error_text());
    }
    if let Some(rest) = step.strip_prefix("… ") {
        return (None, rest, theme::text());
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
    (None, step, theme::dim())
}

/// The color of a tool's badge: reads cool, writes warm, searches in between.
fn tool_color(badge: &str) -> Color32 {
    match badge {
        "read" | "glob" => theme::debug(),
        "edit" | "write" => theme::accent(),
        "grep" | "search" | "fetch" => theme::trace(),
        "plan" => theme::online(),
        _ => theme::dim(),
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
