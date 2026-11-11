//! The Blood Moon theme for the workspace: the same colors the console paints with
//! (`crates/ra-console/src/theme.rs`, from the workbench's `BloodMoon.theme.json`), as egui
//! colors, plus the visuals that make egui's widgets wear them.

use egui::{Color32, CornerRadius, Stroke, Visuals};

pub(crate) const BG: Color32 = Color32::from_rgb(23, 7, 9);
pub(crate) const PANEL: Color32 = Color32::from_rgb(38, 14, 18);
pub(crate) const PANEL_RAISED: Color32 = Color32::from_rgb(48, 18, 23);
pub(crate) const RULE: Color32 = Color32::from_rgb(75, 34, 40);
pub(crate) const DIM: Color32 = Color32::from_rgb(140, 91, 98);
pub(crate) const ACCENT: Color32 = Color32::from_rgb(230, 57, 53);
pub(crate) const TEXT: Color32 = Color32::from_rgb(233, 213, 211);
pub(crate) const ERROR_TEXT: Color32 = Color32::from_rgb(255, 138, 122);
pub(crate) const WARN: Color32 = Color32::from_rgb(210, 153, 34);
pub(crate) const WARN_TEXT: Color32 = Color32::from_rgb(242, 204, 96);
pub(crate) const DEBUG: Color32 = Color32::from_rgb(57, 197, 207);
pub(crate) const TRACE: Color32 = Color32::from_rgb(188, 140, 255);
pub(crate) const ONLINE: Color32 = Color32::from_rgb(63, 185, 80);

/// The lane colors of the tree depth, one vivid color per level of the roadmap.
const LANES: [(u8, u8, u8); 10] = [
    (255, 83, 112),
    (255, 148, 82),
    (255, 214, 102),
    (63, 200, 120),
    (57, 197, 178),
    (87, 214, 224),
    (100, 148, 255),
    (163, 133, 255),
    (255, 121, 198),
    (255, 110, 149),
];

/// The bright lane color of a depth.
pub(crate) fn lane(depth: usize) -> Color32 {
    let (r, g, b) = LANES[depth % LANES.len()];
    Color32::from_rgb(r, g, b)
}

/// The color `t` of the way from `from` to `to`.
pub(crate) fn mix(from: Color32, to: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let step = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
    Color32::from_rgb(step(from.r(), to.r()), step(from.g(), to.g()), step(from.b(), to.b()))
}

/// The color of a task state, as the console's badges have it.
pub(crate) fn state_color(state: ra_roadmap::model::TaskState) -> Color32 {
    use ra_roadmap::model::TaskState as S;
    match state {
        S::Defined => DIM,
        S::Ready | S::Assigned => TRACE,
        S::InProgress | S::Testing | S::Review => WARN_TEXT,
        S::Implemented => DEBUG,
        S::Accepted => ONLINE,
        S::Blocked | S::NeedsRevision => WARN,
        S::Failed | S::Cancelled => ERROR_TEXT,
    }
}

/// The visuals of the workspace: dark surfaces, the red accent, soft corners.
pub(crate) fn visuals() -> Visuals {
    let mut v = Visuals::dark();
    v.override_text_color = Some(TEXT);
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.extreme_bg_color = Color32::from_rgb(16, 5, 7);
    v.faint_bg_color = PANEL;
    v.window_stroke = Stroke::new(1.0, RULE);
    v.selection.bg_fill = Color32::from_rgb(120, 30, 34);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT;
    v.warn_fg_color = WARN_TEXT;
    v.error_fg_color = ERROR_TEXT;
    let widgets = &mut v.widgets;
    widgets.noninteractive.bg_fill = PANEL;
    widgets.noninteractive.weak_bg_fill = PANEL;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, RULE);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, DIM);
    widgets.inactive.bg_fill = PANEL_RAISED;
    widgets.inactive.weak_bg_fill = PANEL_RAISED;
    widgets.inactive.bg_stroke = Stroke::new(1.0, RULE);
    widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    widgets.hovered.bg_fill = Color32::from_rgb(70, 26, 31);
    widgets.hovered.weak_bg_fill = Color32::from_rgb(70, 26, 31);
    widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    widgets.hovered.fg_stroke = Stroke::new(1.5, TEXT);
    widgets.active.bg_fill = Color32::from_rgb(120, 30, 34);
    widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    widgets.active.fg_stroke = Stroke::new(2.0, TEXT);
    widgets.open.bg_fill = PANEL_RAISED;
    widgets.open.bg_stroke = Stroke::new(1.0, RULE);
    for w in [&mut widgets.noninteractive, &mut widgets.inactive, &mut widgets.hovered, &mut widgets.active, &mut widgets.open] {
        w.corner_radius = CornerRadius::same(6);
    }
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v
}
