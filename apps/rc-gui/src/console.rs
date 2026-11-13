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
            Some((id, run)) if line.kind == Kind::Run => run_card(ui, sink, id, &line.text, &run, index, None, None),
            _ => bubble(ui, line, index),
        }
        ui.add_space(6.0);
    }
    for (index, line, id, run) in live {
        run_card(ui, sink, id, &line.text, &run, index, None, None);
        ui.add_space(6.0);
    }
}

/// The rows a run card gets on the dashboard when the queue beside it is shorter: enough to
/// follow the work, never the page.
const DASHBOARD_ROWS: f32 = 3.0;

/// The latest run's card alone, the live one before a finished one, for the dashboard, in
/// `room` points of height at most (the queue beside it sets it); `false` when the session
/// has run nothing yet.
/// The most live run cards a project's dashboard card shows: the whole Opus pool.
pub(crate) const LIVE_CARDS: usize = 12;

/// What a bounded card's frame takes of its room: the margins (5 above and below) and the
/// stroke (1.5 each side).
const CARD_FRAME: f32 = 13.0;

/// The least a live run card is tall: its frame, its title row with the space under it (34,
/// the badges' height) and three step rows of 24 (the fold line and two steps), with two
/// points to spare so the third row is never a fraction short; no more than that, since the
/// blank under the rows is what kept two by six off the screen.
pub(crate) const MIN_CARD_HEIGHT: f32 = CARD_FRAME + 34.0 + 3.0 * 24.0 + 2.0;

/// The columns of the run grid for `n` live cards: two, so twelve cards are six rows of two
/// in one bubble (the person: "needs to support up to 2x6 in one bubble").
pub(crate) fn grid_columns(n: usize) -> usize {
    n.clamp(1, 2)
}

pub(crate) fn draw_latest_run(ui: &mut Ui, sink: &Sink, room: f32, column: f32, project_model: Option<&str>) -> bool {
    let runs: Vec<(usize, u64, String)> = sink.with_lines(|lines| {
        lines.iter().enumerate().filter(|(_, l)| l.kind == Kind::Run).filter_map(|(i, l)| l.run.map(|id| (i, id, l.text.clone()))).collect()
    });
    // the live runs, oldest first, the newest twelve at most (the whole Opus pool, which one
    // project has to itself under a priority order; a run whose process was killed under it
    // stays open in the log until the orphan scan closes it), else the latest run
    let live = sink.live_runs();
    let newest: Vec<u64> = live.iter().rev().take(LIVE_CARDS).rev().copied().collect();
    let picked: Vec<&(usize, u64, String)> = if newest.is_empty() {
        runs.last().into_iter().collect()
    } else {
        newest.iter().filter_map(|id| runs.iter().rev().find(|(_, run, _)| run == id)).collect()
    };
    if picked.is_empty() {
        return false;
    }
    // a fixed room: in a scroll area the clip is the page, and a live card would take it all;
    // the queue beside the card is the most it gets, the default rows the least
    let room = room.max(96.0 + 24.0 * DASHBOARD_ROWS);
    // the live cards in a grid of two columns: two side by side, four two by two, six two by
    // three, each half the column's width and a row's part of its height, less the space
    // between; a project running flat out still fills its share. The column's width comes
    // from the caller, since what egui called available here ran past the column
    // a thin gap: six rows of cards fit the window only with little between them
    let gap = 6.0;
    let cols = grid_columns(picked.len());
    let rows = picked.len().div_ceil(cols.max(1));
    #[allow(clippy::cast_precision_loss)]
    let width = ((column - gap * (cols as f32 - 1.0)) / cols as f32).floor();
    // a card is never shorter than its title, the fold line and two steps: the frame (19),
    // the title row with the space under it (some 36 with the badges' own height) and three
    // rows of 26, with a few points to spare so the third row is never a fraction short; a
    // grid that would squeeze the cards below that grows past its share and the page scrolls
    #[allow(clippy::cast_precision_loss)]
    let height = ((room - gap * (rows as f32 - 1.0)) / rows as f32).floor().max(MIN_CARD_HEIGHT);
    for row in picked.chunks(cols.max(1)) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (index, id, title) in row {
                let Some(run) = sink.run(*id) else { continue };
                // each card gets exactly its cell, clipped to it: a card whose text ran long grew
                // past its half and pushed its neighbour (the person: "should never do that")
                // clipped across only: the card's height is its own (a clip on all sides cut the
                // bottom of the cards off), its width never more than its half
                ui.allocate_ui_with_layout(egui::vec2(width, height), egui::Layout::top_down(egui::Align::Min), |ui| {
                    let left = ui.max_rect().left();
                    let across = egui::Rect::from_x_y_ranges(left - 2.0..=left + width + 2.0, ui.clip_rect().y_range());
                    ui.set_clip_rect(across.intersect(ui.clip_rect()));
                    ui.set_width(width);
                    ui.set_max_width(width);
                    run_card(ui, sink, *id, title, &run, *index, Some(height), project_model);
                });
            }
        });
        if rows > 1 {
            ui.add_space(gap - ui.spacing().item_spacing.y);
        }
    }
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
    // clipped by characters and then by the room: a label wider than the row stretched the
    // card past its column (two cards side by side halve it); truncated, it ends in an
    // ellipsis at the edge and the whole line is on hover
    let count = raw.chars().count();
    let (shown, cut) =
        if count <= limit { (raw.to_owned(), false) } else { (format!("{}…", raw.chars().take(limit).collect::<String>()), true) };
    let label = egui::Label::new(RichText::new(shown).font(font.clone()).color(color)).truncate();
    let response = ui.add(label);
    // the whole line on hover when it may not all be on the row
    if cut || count > 60 {
        response.on_hover_text(RichText::new(raw).font(font.clone()));
    }
}

/// A worker run as a card: the title and the clock or the outcome on top, the latest steps
/// inside at the run's height, a soft light flowing along the border while it is live. The
/// wheel over the card sizes the card, not the log; a click on the header folds the card to
/// that one line, and a finished card folds itself after a minute.
fn run_card(ui: &mut Ui, sink: &Sink, id: u64, title: &str, run: &Run, index: usize, room: Option<f32>, project_model: Option<&str>) {
    let lane = theme::lane(1 + index % 9);
    // the animation runs on the window's clock, not the run's: every live card on the
    // dashboard breathes and its light laps in step, instead of each on its own start time
    #[allow(clippy::cast_possible_truncation)]
    let elapsed = ui.input(|i| i.time) as f32;
    let base = if failed(run) {
        theme::accent()
    } else if run.live() {
        // the border breathes between the rule and the bright lane while the run works
        let pulse = ((elapsed * 1.6).sin() * 0.5 + 0.5) * 0.6 + 0.2;
        theme::mix(theme::rule(), lane, pulse)
    } else {
        lane
    };
    // a bounded card (the dashboard's grid) keeps a thin margin: twelve of them fit the window
    // only with little around each
    let margin_y: i8 = if room.is_some() { 5 } else { 8 };
    let frame = Frame::new()
        .fill(theme::panel())
        .stroke(Stroke::new(1.5, base))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(10, margin_y));
    let response = frame
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let folded = run.folded();
            if run_title(ui, sink, id, title, run, lane, folded, project_model).clicked() {
                sink.toggle_fold(id);
            }
            if folded {
                return;
            }
            ui.add_space(if room.is_some() { 2.0 } else { 4.0 });
            // a live card fills the console it is in, or the room the caller gives it; a
            // finished one shrinks to its cap. Room given is a bound: the card never grows
            // past it, whatever its own height setting
            let bounded = room.is_some();
            // a bounded card's content is its room less the frame's margins (5 above and
            // below) and stroke (1.5); the rows get what is left of that under the title.
            // The console hands the clip
            let want = room.map(|room| (room - CARD_FRAME).floor());
            let rows_room = match want {
                Some(want) => want - ui.min_rect().height(),
                None => ui.clip_rect().height(),
            };
            run_steps(ui, run, rows_room, bounded);
            // a bounded card is exactly its room tall, live or finished: the dashboard gives
            // the run the height of the queue beside it, and the two read as one band
            if let Some(want) = want {
                let used = ui.min_rect().height();
                if want > used {
                    ui.allocate_space(egui::vec2(ui.available_width(), want - used));
                }
            }
        })
        .response;
    // the wheel sizes a card in a project's console; on the dashboard (a bounded card, sized by
    // its grid) the wheel scrolls the page, as anywhere else on it
    if !run.folded() && room.is_none() {
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
fn run_title(
    ui: &mut Ui,
    sink: &Sink,
    id: u64,
    title: &str,
    run: &Run,
    lane: Color32,
    folded: bool,
    project_model: Option<&str>,
) -> Response {
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
    // the row keeps to the card, measured, not guessed: the status (an outcome can be a whole
    // sentence) is cut to a third of the row, the model's badge, the buttons and the badges'
    // frames come off, and the task gets the rest, cut with an ellipsis and whole on hover. A
    // badge does not wrap, and an uncut one pushed the card past its half of the grid
    let small = egui::TextStyle::Small.resolve(ui.style());
    let measure = |ui: &Ui, text: &str| ui.painter().layout_no_wrap(text.to_owned(), small.clone(), Color32::WHITE).size().x;
    let cut_to = |ui: &Ui, text: &str, width: f32| -> (String, bool) {
        if measure(ui, text) <= width {
            return (text.to_owned(), false);
        }
        let chars: Vec<char> = text.chars().collect();
        let (mut lo, mut hi) = (0, chars.len());
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            let candidate: String = chars[..mid].iter().collect::<String>() + "…";
            if measure(ui, &candidate) <= width { lo = mid } else { hi = mid - 1 }
        }
        (chars[..lo].iter().collect::<String>() + "…", true)
    };
    let total = ui.available_width();
    let (status_shown, status_cut) = cut_to(ui, &status, (total / 3.0).max(60.0));
    // the model's badge only when the card runs on another model than its project's: the
    // project's header says that one already (the person: "hide the redundant model badges")
    let model = title.rsplit_once(" · ").map(|(_, model)| model).filter(|m| project_model.is_none_or(|p| !p.eq_ignore_ascii_case(m)));
    let model_width = model.map_or(0.0, |m| measure(ui, m) + 34.0);
    // the fold arrow, the two buttons, the badges' frames and marks, and the spacing between
    let fixed = 14.0 + if folded { 0.0 } else { 52.0 } + 34.0 + 34.0;
    let task_room = (total - measure(ui, &status_shown) - model_width - fixed).max(40.0);
    let task = title.rsplit_once(" · ").map_or(title, |(task, _)| task);
    let row = ui.horizontal(|ui| {
        ui.label(RichText::new(if folded { "▸" } else { "▾" }).color(theme::dim()).small());
        // the task in its room, running as a marquee when it is longer, whole on hover
        crate::cards::marquee_badge(ui, "◇", task, task_room, theme::trace()).on_hover_text(task);
        if let Some(model) = model {
            crate::cards::badge_marked(ui, "◉", model, crate::cards::model_color(model));
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
            // the clock and the steps, or the outcome, as a badge of their own
            let shown = crate::cards::badge(ui, &status_shown, status_color);
            if status_cut {
                shown.on_hover_text(&status);
            }
        });
    });
    // the row is sensed after its buttons, so a click on one of them is theirs, not the fold's
    row.response.interact(Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The body of a run card: the latest steps at the run's height. A live run keeps its height,
/// with the rows it has not filled yet left open; a finished one shrinks to its steps. The
/// steps that fell off the top fold into one line that shows them on hover.
fn run_steps(ui: &mut Ui, run: &Run, room: f32, bounded: bool) {
    let font = FontId::monospace(12.5);
    // a step that repeats the one before folds into it with a count at the end; the rows are
    // counted after the folding, so the card shows as many entries as it has rows for
    let mut folded: Vec<(&str, usize)> = Vec::new();
    for step in &run.steps {
        // a tool call with nothing after its name (a Grep over the whole tree) is no line
        if let (Some(_), rest, _) = step_tool(step)
            && rest.trim().is_empty()
        {
            continue;
        }
        match folded.last_mut() {
            Some((last, count)) if *last == step.as_str() => *count += 1,
            _ => folded.push((step.as_str(), 1)),
        }
    }
    let height = usize::from(run.height);
    // a live run takes the rows the console has room for, the cap at least; a finished one its
    // cap or fewer. In the console the room is the clip rect, and the card's own frame, title,
    // folded line and the spacing under it come off it before the rows: with the rows alone
    // counted the card overshot the view and its top was always cut off
    // A bounded card (the dashboard) gets the height left under its title and pads itself to
    // it after the rows, so the rows are that height over a row's real height: the font's
    // line, the field's margins (2 above and below) and the spacing between rows. A constant
    // of 24 here drew a row or two short and padded the rest blank. The folded line, when
    // the steps overflow, takes one of the rows. The console keeps its clip arithmetic
    // a bounded card's row pitch is the row's geometry: the jewel's 18 points set the field's
    // height (the text is shorter), the field's margins add 2 above and below, and the
    // spacing between rows follows; an estimate from the font ran short of that and the card
    // overflowed the queue beside it
    // a bounded card keeps a thin space between its rows: twelve cards in one bubble fit the
    // window only with the rows close (the person: "tighten up the gap at the bottom")
    if bounded {
        ui.spacing_mut().item_spacing.y = 2.0;
    }
    let row = if bounded { crate::cards::JEWEL_HEIGHT + 4.0 + ui.spacing().item_spacing.y } else { 24.0 };
    let overhead = if bounded { 0.0 } else { 136.0 };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let fit = ((room - overhead) / row).floor().max(0.0) as usize;
    let body = if bounded {
        // the dashboard: the rows the room holds; when steps fell off the top the folded line
        // is one of them (the skip below counts it), and a run with fewer steps than rows
        // shows them all and pads to its room
        let rows = fit.max(1);
        if folded.len() > rows { rows } else { folded.len().max(1) }
    } else if run.live() {
        height.max(fit)
    } else {
        height.min(folded.len().max(1))
    };
    let hidden = folded.len().saturating_sub(body);
    // a bounded card spreads what is left under a whole number of rows between them, so the
    // rows end on the card's bottom edge instead of a part of a row short of it; the spacing
    // is this card's alone
    if bounded {
        // the rows drawn: the folded line stands in for one of the body's when steps hid
        let rows = f32::from(u16::try_from(body.max(1)).unwrap_or(u16::MAX));
        // a thin space at most: with few lines the rest of the room stays under them rather
        // than spread them apart
        let extra = ((room - rows * row) / rows).clamp(0.0, 3.0);
        // on the pixel grid: a fraction of a pixel between rows put the text between pixels,
        // and it snapped up or down a pixel from frame to frame
        let ppp = ui.pixels_per_point();
        ui.spacing_mut().item_spacing.y += (extra * ppp).floor() / ppp;
    }
    // the entries that fell off the top fold into one line, which takes a row of its own;
    // a card with one row shows its latest step, not the fold alone (six cards in a short
    // grid read "earlier steps" and nothing else)
    let fold = hidden > 0 && body >= 2;
    let skip = if fold { hidden + 1 } else { hidden };
    if fold {
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
    // a live card in the console keeps its rows open; a bounded card pads to its room instead
    if run.live() && !bounded {
        for _ in drawn..body {
            ui.label(RichText::new(" ").font(font.clone()));
        }
    }
}

/// The tools a step names and the badge each wears.
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
        let carried = data.get_temp_mut_or_default::<f32>(key);
        *carried += delta;
        while *carried >= NOTCH {
            *carried -= NOTCH;
            rows += 2;
        }
        while *carried <= -NOTCH {
            *carried += NOTCH;
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
