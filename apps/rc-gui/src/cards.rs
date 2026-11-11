//! The visual vocabulary of the workspace: glowing cards, badges, priority dots, section
//! headers, and the rich tooltip of a roadmap card. Everything takes a lane color, so a card
//! carries its depth's color wherever it shows, the way the console's tree does.

use egui::epaint::RectShape;
use egui::{Color32, CornerRadius, Frame, InnerResponse, Margin, Painter, Rect, RichText, Stroke, Ui};
use ra_roadmap::model::{Node, NodeKind};

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
    let top = alpha(Color32::WHITE, strength);
    let bottom = alpha(Color32::WHITE, 0);
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
        .fill(theme::PANEL)
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
        .fill(theme::mix(theme::BG, color, 0.07))
        .stroke(Stroke::new(1.0, alpha(color, if lit { 245 } else { 175 })))
        .corner_radius(CornerRadius::same(9))
        .inner_margin(Margin::symmetric(8, 6))
        .outer_margin(Margin::symmetric(0, 3));
    let painter = ui.painter().clone();
    let slot = painter.add(egui::Shape::Noop);
    let response = frame.show(ui, contents);
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
                ui.label(RichText::new(text).color(theme::TEXT).small());
            });
        })
        .response
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
    painter.text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(11.0), theme::TEXT);
    response
}

/// A filter chip: a badge that is lit while `on` and dim while off, and reports a click.
pub(crate) fn chip(ui: &mut Ui, text: &str, color: Color32, on: bool) -> bool {
    let response = Frame::new()
        .fill(alpha(color, if on { 40 } else { 8 }))
        .stroke(Stroke::new(1.0, alpha(color, if on { 180 } else { 60 })))
        .corner_radius(CornerRadius::same(9))
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(if on { color } else { alpha(theme::DIM, 170) }).small().strong());
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
            ui.label(RichText::new(text).color(if on { color } else { alpha(theme::DIM, 190) }).small().strong());
        })
        .response
        .interact(egui::Sense::click());
    response.clicked()
}

/// The priority of a card as five dots, `complexity` of them lit, warm to hot.
pub(crate) fn priority(ui: &mut Ui, complexity: u8) -> egui::Response {
    let lit = usize::from(complexity.clamp(1, 5));
    let hot = theme::mix(theme::WARN_TEXT, theme::ACCENT, (lit as f32 - 1.0) / 4.0);
    let response = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for i in 0..5 {
                let color = if i < lit { hot } else { alpha(theme::DIM, 90) };
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
/// its own color, so the card reads at a glance without a click.
pub(crate) fn tooltip(ui: &mut Ui, node: &Node, lane: Color32) {
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
        ui.label(RichText::new(format!("#{} · {}", node.id, node.kind)).color(theme::DIM).small());
        priority(ui, node.complexity);
        if node.is_final() {
            badge(ui, "final", theme::WARN);
        }
    });
    if let Some(objective) = &node.objective {
        section(ui, "OBJECTIVE", theme::WARN_TEXT);
        ui.label(RichText::new(objective).color(theme::TEXT));
    }
    if let Some(acceptance) = &node.acceptance {
        section(ui, "ACCEPTANCE", theme::ONLINE);
        ui.label(RichText::new(acceptance).color(theme::mix(theme::TEXT, theme::ONLINE, 0.25)));
    }
    let model = node.assignee.as_ref().map_or("badge model", |a| a.model.as_str());
    ui.horizontal_wrapped(|ui| {
        badge_marked(ui, "◉", model, theme::TRACE);
        if !node.files.is_empty() {
            badge_marked(ui, "▤", &format!("{} files", node.files.len()), theme::DEBUG);
        }
        if node.session.is_some() {
            badge_marked(ui, "↻", "resumes", theme::ONLINE);
        }
        if !node.depends_on.is_empty() {
            badge_marked(
                ui,
                "⇠",
                &format!("waits on {}", node.depends_on.iter().map(|d| format!("#{d}")).collect::<Vec<_>>().join(" ")),
                theme::DIM,
            );
        }
    });
    if let Some(first) = node.notes.as_deref().and_then(|n| n.lines().find(|l| !l.trim().is_empty())) {
        section(ui, "NOTES", theme::DIM);
        ui.label(RichText::new(first.chars().take(240).collect::<String>()).color(theme::DIM).italics());
    }
    let rect = ui.min_rect().expand(6.0);
    painter.set(slot, RectShape::filled(rect.expand(7.0), CornerRadius::same(12), alpha(lane, 40)).with_blur_width(28.0));
}
