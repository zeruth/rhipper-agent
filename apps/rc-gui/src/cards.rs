//! The visual vocabulary of the workspace: glowing cards, badges, priority dots, section
//! headers, and the rich tooltip of a roadmap card. Everything takes a lane color, so a card
//! carries its depth's color wherever it shows, the way the console's tree does.

use egui::epaint::RectShape;
use egui::{Align, Color32, CornerRadius, Frame, InnerResponse, Layout, Margin, Painter, Rect, RichText, Stroke, Ui};
use ra_roadmap::history::{self, Change};
use ra_roadmap::model::{Gambit, GambitState, Node, NodeKind, TaskState};

use crate::theme;

/// A color with another alpha.
pub(crate) fn alpha(color: Color32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), a)
}

/// Paints a soft glow of `color` behind `rect`: a blurred, rounded rectangle a little larger.
pub(crate) fn glow(painter: &Painter, rect: Rect, color: Color32, strength: u8, spread: f32) {
    let shape = RectShape::filled(rect.expand(spread * 0.5), CornerRadius::same(12), alpha(color, strength)).with_blur_width(spread * 2.0);
    painter.add(shape);
}

/// A sheen across the top of `rect`: a band of light that fades down, the gradient of a raised
/// surface, kept inside the rounded corners.
pub(crate) fn sheen(painter: &Painter, rect: Rect, strength: u8) {
    use egui::epaint::{Mesh, Vertex, WHITE_UV};
    let band =
        Rect::from_min_max(rect.left_top() + egui::vec2(10.0, 1.0), egui::pos2(rect.right() - 10.0, rect.top() + rect.height() * 0.45));
    let top = alpha(theme::light(), strength);
    let bottom = alpha(theme::light(), 0);
    let mut mesh = Mesh::default();
    let i = mesh.vertices.len() as u32;
    mesh.vertices.push(Vertex { pos: band.left_top(), uv: WHITE_UV, color: top });
    mesh.vertices.push(Vertex { pos: band.right_top(), uv: WHITE_UV, color: top });
    mesh.vertices.push(Vertex { pos: band.right_bottom(), uv: WHITE_UV, color: bottom });
    mesh.vertices.push(Vertex { pos: band.left_bottom(), uv: WHITE_UV, color: bottom });
    mesh.add_triangle(i, i + 1, i + 2);
    mesh.add_triangle(i, i + 2, i + 3);
    painter.add(mesh);
}

/// A card: a raised panel with a glow and a stroke in `color`, a sheen across its top, and
/// `contents` inside. `lit` makes the glow strong, for the selected or the hovered card.
pub(crate) fn card<R>(ui: &mut Ui, color: Color32, lit: bool, contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let frame = Frame::new()
        .fill(theme::panel())
        .stroke(Stroke::new(1.0, alpha(color, if lit { 220 } else { 110 })))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(12, 10));
    // the glow goes under the frame: a slot is taken before the frame paints and filled once
    // the rect is known; the sheen goes over it, as light does
    let painter = ui.painter().clone();
    let slot = painter.add(egui::Shape::Noop);
    let response = frame.show(ui, contents);
    let rect = response.response.rect;
    let spread = if lit { 10.0 } else { 6.0 };
    let shape = RectShape::filled(rect.expand(spread * 0.5), CornerRadius::same(12), alpha(color, if lit { 90 } else { 40 }))
        .with_blur_width(spread * 2.0);
    painter.set(slot, shape);
    sheen(&painter, rect, 10);
    response
}

/// A bubble of the roadmap column: a card with a tighter margin, tinted a little toward its
/// lane so a bubble inside a bubble reads as one, with its glow and its sheen. `lit` brightens
/// it, for the bubble that holds the selection.
pub(crate) fn bubble<R>(ui: &mut Ui, color: Color32, lit: bool, contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    // dark, with a breath of the lane in it; the outline carries the lane bright, so the depth
    // reads from the colors the way the console's frames do
    let frame = Frame::new()
        .fill(theme::mix(theme::bg(), color, 0.07))
        .stroke(Stroke::new(1.0, alpha(color, if lit { 245 } else { 175 })))
        .corner_radius(CornerRadius::same(9))
        .inner_margin(Margin::symmetric(8, 6))
        .outer_margin(Margin::symmetric(0, 3));
    let painter = ui.painter().clone();
    let slot = painter.add(egui::Shape::Noop);
    // the contents read top to bottom whatever row the bubble sits in: a frame takes its
    // parent's layout, and in a row of bubbles that laid a header beside its own body
    let response = frame.show(ui, |ui| ui.with_layout(Layout::top_down(Align::Min), contents).inner);
    let rect = response.response.rect;
    let spread = if lit { 8.0 } else { 4.0 };
    let shape = RectShape::filled(rect.expand(spread * 0.5), CornerRadius::same(11), alpha(color, if lit { 80 } else { 30 }))
        .with_blur_width(spread * 2.0);
    painter.set(slot, shape);
    sheen(&painter, rect, 8);
    response
}

/// A glowing rule across the width, between the bubbles of two phases.
pub(crate) fn divider(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 14.0), egui::Sense::hover());
    let y = rect.center().y;
    let line = Rect::from_min_max(egui::pos2(rect.left() + 10.0, y - 0.5), egui::pos2(rect.right() - 10.0, y + 0.5));
    let painter = ui.painter();
    painter.add(RectShape::filled(line.expand2(egui::vec2(0.0, 3.0)), CornerRadius::same(3), alpha(color, 70)).with_blur_width(8.0));
    painter.rect_filled(line, CornerRadius::ZERO, alpha(color, 170));
}

/// A badge: a small pill with `text` in `color` on a tinted field.
pub(crate) fn badge(ui: &mut Ui, text: &str, color: Color32) -> egui::Response {
    Frame::new()
        .fill(alpha(color, 28))
        .stroke(Stroke::new(1.0, alpha(color, 160)))
        .corner_radius(CornerRadius::same(9))
        .inner_margin(Margin::symmetric(7, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(color).small().strong());
        })
        .response
}

/// A jewel: a fixed-width tag that is bright in `color` at its left and fades to nothing by its
/// right edge, the label set in the bright part. Every jewel in a column has the same width,
/// so the text after them lines up whatever the label's length; the fade is the padding.
/// The height of a jewel, which sets the height of a step row on a run card.
pub(crate) const JEWEL_HEIGHT: f32 = 18.0;

pub(crate) fn jewel(ui: &mut Ui, text: &str, color: Color32, width: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, JEWEL_HEIGHT), egui::Sense::hover());
    let painter = ui.painter();
    fade(painter, rect, color, 120, false);
    // a bright edge at the left, the facet the light catches
    painter.rect_filled(Rect::from_min_size(rect.left_top(), egui::vec2(2.0, rect.height())), CornerRadius::same(1), alpha(color, 230));
    painter.text(
        egui::pos2(rect.left() + 8.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::proportional(11.5),
        theme::mix(theme::text(), color, 0.35),
    );
}

/// A horizontal fade across `rect`: `strength` of `color` at one edge, nothing at the other;
/// bright at the right with `from_right`, at the left otherwise. The gradient of a jewel, and
/// of a roadmap row, where the light sits by the state.
pub(crate) fn fade(painter: &Painter, rect: Rect, color: Color32, strength: u8, from_right: bool) {
    painter.add(fade_shape(rect, color, strength, from_right));
}

/// The fade as a shape, for a slot taken before the thing it goes under is drawn.
pub(crate) fn fade_shape(rect: Rect, color: Color32, strength: u8, from_right: bool) -> egui::Shape {
    use egui::epaint::{Mesh, Vertex, WHITE_UV};
    let (bright, gone) = (alpha(color, strength), alpha(color, 0));
    let (left, right) = if from_right { (gone, bright) } else { (bright, gone) };
    let mut mesh = Mesh::default();
    mesh.vertices.push(Vertex { pos: rect.left_top(), uv: WHITE_UV, color: left });
    mesh.vertices.push(Vertex { pos: rect.right_top(), uv: WHITE_UV, color: right });
    mesh.vertices.push(Vertex { pos: rect.right_bottom(), uv: WHITE_UV, color: right });
    mesh.vertices.push(Vertex { pos: rect.left_bottom(), uv: WHITE_UV, color: left });
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    egui::Shape::mesh(mesh)
}

/// A badge with a mark in front of the text.
pub(crate) fn badge_marked(ui: &mut Ui, mark: &str, text: &str, color: Color32) -> egui::Response {
    Frame::new()
        .fill(alpha(color, 28))
        .stroke(Stroke::new(1.0, alpha(color, 160)))
        .corner_radius(CornerRadius::same(9))
        .inner_margin(Margin::symmetric(7, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(RichText::new(mark).color(color).small());
                ui.label(RichText::new(text).color(theme::text()).small());
            });
        })
        .response
}

/// A toast: a card the width of a note, the run's name in `color` over its outcome, that
/// answers a click on its whole face. A failed run's is lit and its outcome reads as an error;
/// an outcome that counts gotchas reads as a warning.
pub(crate) fn toast(ui: &mut Ui, name: &str, outcome: &str, color: Color32, failed: bool) -> egui::Response {
    let tone = if failed {
        theme::error_text()
    } else if outcome.contains("gotcha") {
        theme::warn_text()
    } else {
        theme::text()
    };
    let response = card(ui, color, failed, |ui| {
        ui.set_width(320.0);
        ui.label(RichText::new(name).color(color).strong());
        ui.label(RichText::new(outcome).color(tone).small());
    })
    .response
    .interact(egui::Sense::click());
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// A pill bar: a badge-sized pill filled to `fill` (0 to 1) in `color`, with `text` across it.
/// `pulse` is the strength of the glow behind the fill: a rest value most of the time, a
/// brighter one right after the value moved, so a change is seen.
pub(crate) fn pill_bar(ui: &mut Ui, text: &str, fill: f32, color: Color32, pulse: u8) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(132.0, 20.0), egui::Sense::hover());
    let painter = ui.painter();
    let radius = CornerRadius::same(10);
    painter.rect(rect, radius, alpha(color, 16), Stroke::new(1.0, alpha(color, 150)), egui::StrokeKind::Inside);
    let fill = fill.clamp(0.0, 1.0);
    if fill > 0.0 {
        // the fill never narrows below its own rounding, so a few percent still reads as a pill
        let filled = Rect::from_min_size(rect.min, egui::vec2((rect.width() * fill).max(20.0), rect.height()));
        glow(painter, filled, color, pulse, 6.0);
        painter.rect_filled(filled, radius, alpha(color, 110 + pulse / 2));
        sheen(painter, filled, 36);
    }
    painter.text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(11.0), theme::text());
    response
}

/// A sparkline of the week: one dot per day, oldest left, joined where days neighbour, over a
/// faint baseline; a day without a reading leaves a gap. The values run 0 to 1.
pub(crate) fn sparkline(ui: &mut Ui, days: &[Option<f64>; 7], color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(126.0, 28.0), egui::Sense::hover());
    let painter = ui.painter();
    let plot = rect.shrink2(egui::vec2(4.0, 4.0));
    painter.hline(plot.x_range(), plot.bottom(), Stroke::new(1.0, alpha(color, 60)));
    let step = plot.width() / 6.0;
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    let point = |slot: usize, value: f64| {
        egui::pos2(plot.left() + step * slot as f32, plot.bottom() - plot.height() * (value as f32).clamp(0.0, 1.0))
    };
    let mut previous: Option<egui::Pos2> = None;
    for (slot, day) in days.iter().enumerate() {
        let Some(value) = day else {
            previous = None;
            continue;
        };
        let at = point(slot, *value);
        if let Some(from) = previous {
            painter.line_segment([from, at], Stroke::new(1.5, alpha(color, 200)));
        }
        painter.circle_filled(at, 2.0, color);
        previous = Some(at);
    }
}

/// How many samples a machine gauge keeps and its graph spans: two minutes, one a second.
pub(crate) const GAUGE_SPAN: usize = 120;

/// A machine gauge of the dashboard: a bubble `width` wide with `name` and the `value` as a
/// badge across its top, and under them `samples` (0 to 1, the newest last) as a filled area
/// under a line in `color`, the oldest end fading into the bubble's fill; a thin rule marks the
/// top of the range.
#[allow(clippy::cast_possible_truncation)]
/// One build slot of the machine as a slim bubble `width` wide: `BUILDER n` and, while a build
/// holds it, who (`runelite #42`) and for how long, lit in the lane of the slot; dim and `idle`
/// when it is free.
pub(crate) fn builder(ui: &mut Ui, index: usize, held: Option<(&str, u64)>, width: f32) {
    let color = if held.is_some() { theme::lane(7 + index % 3) } else { theme::dim() };
    bubble(ui, color, held.is_some(), |ui| {
        let inner = (width - 18.0).max(60.0);
        ui.set_width(inner);
        ui.set_max_width(inner);
        ui.horizontal(|ui| {
            ui.set_width(inner);
            ui.label(RichText::new(format!("BUILDER {}", index + 1)).small().strong().color(theme::dim()));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| match held {
                Some((who, secs)) => {
                    badge(ui, &format!("{who} · {}:{:02}", secs / 60, secs % 60), color);
                }
                None => {
                    ui.label(RichText::new("idle").small().color(theme::dim()));
                }
            });
        });
    });
}

pub(crate) fn gauge(ui: &mut Ui, name: &str, value: &str, samples: &[f32], color: Color32, width: f32) {
    use egui::epaint::{Mesh, Vertex, WHITE_UV};
    let fill = theme::mix(theme::bg(), color, 0.07);
    bubble(ui, color, false, |ui| {
        // the bubble's inner width: the card less its margins and its stroke, two of each
        let inner = (width - 18.0).max(60.0);
        ui.set_width(inner);
        ui.set_max_width(inner);
        ui.horizontal(|ui| {
            ui.set_width(inner);
            ui.label(RichText::new(name).small().strong().color(theme::dim()));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                badge(ui, value, color);
            });
        });
        // a short plot: the gauges are a glance, the room under them is the projects' (six
        // in one row, slim enough that two by six run cards fit under them)
        let (rect, _) = ui.allocate_exact_size(egui::vec2(inner, 14.0), egui::Sense::hover());
        let painter = ui.painter();
        let plot = rect.shrink2(egui::vec2(0.0, 2.0));
        painter.hline(plot.x_range(), plot.top(), Stroke::new(1.0, alpha(color, 60)));
        let points = gauge_points(samples, plot);
        if points.len() < 2 {
            return;
        }
        // the area under the line: a strip of quads down to the floor of the plot
        let shade = alpha(color, 70);
        let mut mesh = Mesh::default();
        for pair in points.windows(2) {
            let i = mesh.vertices.len() as u32;
            mesh.vertices.push(Vertex { pos: pair[0], uv: WHITE_UV, color: shade });
            mesh.vertices.push(Vertex { pos: pair[1], uv: WHITE_UV, color: shade });
            mesh.vertices.push(Vertex { pos: egui::pos2(pair[1].x, plot.bottom()), uv: WHITE_UV, color: shade });
            mesh.vertices.push(Vertex { pos: egui::pos2(pair[0].x, plot.bottom()), uv: WHITE_UV, color: shade });
            mesh.add_triangle(i, i + 1, i + 2);
            mesh.add_triangle(i, i + 2, i + 3);
        }
        painter.add(egui::Shape::mesh(mesh));
        painter.add(egui::Shape::line(points, Stroke::new(1.5, alpha(color, 220))));
        // the past dissolves into the bubble: the oldest third fades from the fill at the left
        let past = Rect::from_min_max(plot.left_top(), egui::pos2(plot.left() + plot.width() * 0.35, plot.bottom()));
        painter.add(fade_shape(past, fill, 235, false));
    });
}

/// The samples of a gauge as points across `plot`: one slot a second over the span, the newest
/// at the right edge, a full reading at the top. Fewer samples than the span leave the left
/// of the plot empty, so a graph grows in from the right.
#[allow(clippy::cast_precision_loss)]
pub(crate) fn gauge_points(samples: &[f32], plot: Rect) -> Vec<egui::Pos2> {
    // the samples span the whole plot from the first minute on: a young trace stretches,
    // a full one scrolls, and the line never sits in one corner of the card
    let n = samples.len();
    let step = plot.width() / (n.clamp(2, GAUGE_SPAN) - 1) as f32;
    samples
        .iter()
        .enumerate()
        .map(|(i, v)| egui::pos2(plot.right() - (n - 1 - i) as f32 * step, plot.bottom() - plot.height() * v.clamp(0.0, 1.0)))
        .collect()
}

/// A filter chip: a badge that is lit while `on` and dim while off, and reports a click.
pub(crate) fn chip(ui: &mut Ui, text: &str, color: Color32, on: bool) -> bool {
    let response = Frame::new()
        .fill(alpha(color, if on { 40 } else { 8 }))
        .stroke(Stroke::new(1.0, alpha(color, if on { 180 } else { 60 })))
        .corner_radius(CornerRadius::same(9))
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(if on { color } else { alpha(theme::dim(), 170) }).small().strong());
        })
        .response
        .interact(egui::Sense::click());
    response.on_hover_text(if on { "shown · click to hide" } else { "hidden · click to show" }).clicked()
}

/// A tab of the card area: lit while `on`, dim and clickable while not; reports a click.
pub(crate) fn tab(ui: &mut Ui, text: &str, color: Color32, on: bool) -> bool {
    let response = Frame::new()
        .fill(alpha(color, if on { 40 } else { 0 }))
        .stroke(Stroke::new(1.0, alpha(color, if on { 180 } else { 50 })))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(12, 4))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(if on { color } else { alpha(theme::dim(), 190) }).small().strong());
        })
        .response
        .interact(egui::Sense::click());
    response.clicked()
}

/// The priority of a card as five dots, `complexity` of them lit, warm to hot.
pub(crate) fn priority(ui: &mut Ui, complexity: u8) -> egui::Response {
    let lit = usize::from(complexity.clamp(1, 5));
    let hot = theme::mix(theme::warn_text(), theme::accent(), (lit as f32 - 1.0) / 4.0);
    let response = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for i in 0..5 {
                let color = if i < lit { hot } else { alpha(theme::dim(), 90) };
                ui.label(RichText::new("●").color(color).small());
            }
        })
        .response;
    response.on_hover_text(format!("complexity {complexity} of 5"))
}

/// The dollar budget of a complexity, as the runner sets it while budgets are on.
pub(crate) fn budget_text(complexity: u8) -> String {
    let budget = match complexity {
        0 | 1 => 0.25,
        2 => 0.50,
        3 => 1.00,
        4 => 2.00,
        _ => 5.00,
    };
    format!("${budget:.2}")
}

/// A section header: small caps in `color`, with a hairline fading to the right.
pub(crate) fn section(ui: &mut Ui, name: &str, color: Color32) {
    ui.add_space(4.0);
    let response = ui.label(RichText::new(name).small().strong().color(color));
    let y = response.rect.center().y;
    let from = egui::pos2(response.rect.right() + 8.0, y);
    let to = egui::pos2(ui.max_rect().right(), y);
    ui.painter().line_segment([from, to], Stroke::new(1.0, alpha(color, 70)));
}

/// The mark of a kind: a glyph that tells a project from a task at a glance.
pub(crate) fn kind_mark(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Project => "◆",
        NodeKind::Milestone => "◈",
        NodeKind::Section => "▣",
        NodeKind::Task => "●",
        NodeKind::Subtask => "○",
    }
}

/// The rich tooltip of a roadmap card: the title in its lane color, the state and the priority,
/// the objective, the acceptance, the model, the files and the first line of the notes, each in
/// its own color, so the card reads at a glance without a click. With its last state change,
/// who made it and how long the card has sat in its state since, counted from `now`.
pub(crate) fn tooltip(ui: &mut Ui, node: &Node, lane: Color32, last: Option<&Change>, now: u64) {
    ui.set_max_width(480.0);
    ui.spacing_mut().item_spacing.y = 6.0;
    // the glow fits what the tooltip comes to, not the room it was given: a slot is taken now
    // and filled once the content has its rect
    let painter = ui.painter().clone();
    let slot = painter.add(egui::Shape::Noop);
    ui.horizontal(|ui| {
        ui.label(RichText::new(kind_mark(node.kind)).color(lane).size(16.0));
        ui.label(RichText::new(&node.title).color(lane).size(16.0).strong());
    });
    ui.horizontal_wrapped(|ui| {
        badge(ui, &node.state.to_string(), theme::state_color(node.state));
        ui.label(RichText::new(format!("#{} · {}", node.id, node.kind)).color(theme::dim()).small());
        priority(ui, node.complexity);
        if node.is_final() {
            badge(ui, "final", theme::warn());
        }
    });
    // a change recorded for another state than the card's came before a hand edit of the
    // file: the age still counts from it, since nothing later is known
    if let Some(last) = last {
        let since = history::age(now.saturating_sub(last.at));
        let text = if last.to == node.state {
            format!("{} → {} by {} · in {} for {since}", last.from, last.to, last.by, last.to)
        } else {
            format!("{} → {} by {} · {since} ago, edited since", last.from, last.to, last.by)
        };
        ui.label(RichText::new(text).color(theme::dim()).small());
    }
    if let Some(objective) = &node.objective {
        section(ui, "OBJECTIVE", theme::warn_text());
        ui.label(RichText::new(objective).color(theme::text()));
    }
    if let Some(acceptance) = &node.acceptance {
        section(ui, "ACCEPTANCE", theme::online());
        ui.label(RichText::new(acceptance).color(theme::mix(theme::text(), theme::online(), 0.25)));
    }
    let model = node.assignee.as_ref().map_or("badge model", |a| a.model.as_str());
    ui.horizontal_wrapped(|ui| {
        badge_marked(ui, "◉", model, theme::trace());
        if !node.files.is_empty() {
            badge_marked(ui, "▤", &format!("{} files", node.files.len()), theme::debug());
        }
        if node.session.is_some() {
            badge_marked(ui, "↻", "resumes", theme::online());
        }
        if !node.depends_on.is_empty() {
            badge_marked(
                ui,
                "⇠",
                &format!("waits on {}", node.depends_on.iter().map(|d| format!("#{d}")).collect::<Vec<_>>().join(" ")),
                theme::dim(),
            );
        }
    });
    if let Some(first) = node.notes.as_deref().and_then(|n| n.lines().find(|l| !l.trim().is_empty())) {
        section(ui, "NOTES", theme::dim());
        ui.label(RichText::new(first.chars().take(240).collect::<String>()).color(theme::dim()).italics());
    }
    // the glow under it, and a bold outline in the lane on top: the tooltip stands over the
    // rows behind it
    let rect = ui.min_rect().expand(6.0);
    painter.set(slot, RectShape::filled(rect.expand(7.0), CornerRadius::same(12), alpha(lane, 60)).with_blur_width(28.0));
    painter.rect_stroke(rect, CornerRadius::same(9), Stroke::new(2.0, alpha(lane, 255)), egui::StrokeKind::Outside);
}

/// What a row's menu was asked for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RowPick {
    State(TaskState),
    Run,
    Reassess,
    /// Pin the card final, or unpin it when it is.
    PinFinal,
    CopyId,
}

/// The menu a right click opens on a roadmap row: the twelve states, each in its color and the
/// card's own marked, then the actions the card view offers. The pick closes the menu.
pub(crate) fn row_menu(ui: &mut Ui, node: &Node) -> Option<RowPick> {
    ui.set_min_width(160.0);
    ui.label(RichText::new(format!("#{} · {}", node.id, node.kind)).color(theme::dim()).small());
    let mut pick = None;
    for state in TaskState::ALL {
        let color = theme::state_color(state);
        let mark = if state == node.state { "●" } else { "○" };
        let text = RichText::new(format!("{mark}  {state}")).color(if state == node.state {
            color
        } else {
            theme::mix(theme::text(), color, 0.6)
        });
        if ui.button(text).clicked() {
            pick = Some(RowPick::State(state));
        }
    }
    ui.separator();
    if node.kind != NodeKind::Project && ui.button(RichText::new("▶  run").color(theme::accent())).clicked() {
        pick = Some(RowPick::Run);
    }
    if ui.button(RichText::new("↻  reassess").color(theme::warn_text())).clicked() {
        pick = Some(RowPick::Reassess);
    }
    let pin = if node.is_final() { "⊘  unpin" } else { "⊙  pin final" };
    if ui.button(RichText::new(pin).color(theme::warn())).clicked() {
        pick = Some(RowPick::PinFinal);
    }
    if ui.button(RichText::new("⎘  copy id").color(theme::dim())).clicked() {
        pick = Some(RowPick::CopyId);
    }
    if pick.is_some() {
        ui.close();
    }
    pick
}

/// What a gambit bubble was asked for: a click flips the gambit between active and parked,
/// its menu sets the weight or marks it done.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum GambitPick {
    State(GambitState),
    Weight(u8),
}

/// The color of a gambit's state: an active one carries the accent, a parked one waits in the
/// warning tint, a done one is dim.
pub(crate) fn gambit_color(state: GambitState) -> Color32 {
    match state {
        GambitState::Active => theme::accent(),
        GambitState::Parked => theme::warn_text(),
        GambitState::Done => theme::dim(),
    }
}

/// The weight of a gambit as five dots, `weight` of them lit, in the same warm-to-hot run as a
/// card's complexity dots.
pub(crate) fn weight(ui: &mut Ui, weight: u8) -> egui::Response {
    let lit = usize::from(weight.clamp(1, 5));
    let hot = theme::mix(theme::warn_text(), theme::accent(), (f32::from(weight.clamp(1, 5)) - 1.0) / 4.0);
    let response = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for i in 0..5 {
                let color = if i < lit { hot } else { alpha(theme::dim(), 90) };
                ui.label(RichText::new("●").color(color).small());
            }
        })
        .response;
    response.on_hover_text(format!("weight {weight} of 5"))
}

/// A gambit as a bubble in the roadmap column's vocabulary: its number, its text, its weight
/// dots and a state badge. A click parks an active one or activates a parked or done one; a
/// right click opens the menu of weights and the done mark. The pick is the caller's to write.
pub(crate) fn gambit_bubble(ui: &mut Ui, gambit: &Gambit) -> Option<GambitPick> {
    let color = gambit_color(gambit.state);
    let active = gambit.state == GambitState::Active;
    // one gambit a row, the width of the column: the text truncates short of the weight and
    // the state at the right rather than carrying the column past its edge. The width is read
    // inside the frame, where its margins and its stroke are already off: a content set to
    // "the column less the margins" came out two stroke widths wider than the column, and a
    // resizable panel takes its content's width, so the column grew every frame
    let inner = bubble(ui, color, active, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(RichText::new(format!("g{}", gambit.id)).small().color(theme::dim()));
            let text = RichText::new(&gambit.text).color(if gambit.state == GambitState::Done { theme::dim() } else { theme::text() });
            let text = if gambit.state == GambitState::Done { text.strikethrough() } else { text };
            let room = (ui.available_width() - 150.0).max(80.0);
            ui.scope(|ui| {
                ui.set_max_width(room);
                ui.add(egui::Label::new(text).truncate());
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                badge(ui, gambit.state.serde_name(), color);
                weight(ui, gambit.weight);
            });
        });
    });
    let response = ui.interact(inner.response.rect, ui.scope_id().with(("gambit", gambit.id)), egui::Sense::click());
    let hint = if gambit.focus.is_empty() {
        "click parks or activates; right click sets the weight or marks it done".to_owned()
    } else {
        format!("focus: {}\nclick parks or activates; right click sets the weight or marks it done", gambit.focus.join(" "))
    };
    let response = response.on_hover_text(hint);
    let mut pick = None;
    if response.clicked() {
        pick = Some(GambitPick::State(if active { GambitState::Parked } else { GambitState::Active }));
    }
    response.context_menu(|ui| {
        ui.set_min_width(140.0);
        ui.label(RichText::new(format!("g{} · {}", gambit.id, gambit.state)).color(theme::dim()).small());
        for w in 1..=5u8 {
            let mark = if w == gambit.weight { "●" } else { "○" };
            if ui.button(RichText::new(format!("{mark}  weight {w}")).color(theme::text())).clicked() {
                pick = Some(GambitPick::Weight(w));
            }
        }
        ui.separator();
        for state in GambitState::ALL {
            let tint = gambit_color(state);
            let mark = if state == gambit.state { "●" } else { "○" };
            let text = RichText::new(format!("{mark}  {state}")).color(if state == gambit.state {
                tint
            } else {
                theme::mix(theme::text(), tint, 0.6)
            });
            if ui.button(text).clicked() {
                pick = Some(GambitPick::State(state));
            }
        }
        if pick.is_some() {
            ui.close();
        }
    });
    pick
}
