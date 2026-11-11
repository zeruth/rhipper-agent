//! The themes of the workspace: Blood Moon, the same colors the console paints with
//! (`crates/ra-console/src/theme.rs`, from the workbench's `BloodMoon.theme.json`), and
//! Daybreak, a light palette beside it. One theme is current at a time; the window reads every
//! color through the accessors here, so `theme <name>` repaints it whole.

use std::sync::atomic::{AtomicUsize, Ordering};

use egui::{Color32, CornerRadius, Stroke, Visuals};

#[cfg(test)]
mod tests;

/// A palette: the surfaces, the text, the accent, the five log colors and the lanes of the
/// tree depth, one vivid color per level of the roadmap.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Theme {
    pub(crate) name: &'static str,
    pub(crate) dark: bool,
    pub(crate) bg: Color32,
    pub(crate) panel: Color32,
    pub(crate) panel_raised: Color32,
    pub(crate) rule: Color32,
    pub(crate) dim: Color32,
    pub(crate) accent: Color32,
    pub(crate) text: Color32,
    pub(crate) error_text: Color32,
    pub(crate) warn: Color32,
    pub(crate) warn_text: Color32,
    pub(crate) debug: Color32,
    pub(crate) trace: Color32,
    pub(crate) online: Color32,
    /// The deepest surface: a text field's well, and what a step's field sinks toward.
    pub(crate) deep: Color32,
    /// The fill of a selection and of a pressed widget.
    pub(crate) selection: Color32,
    /// The fill of a hovered widget.
    pub(crate) hover: Color32,
    /// The light a sheen and a bold word lean toward.
    pub(crate) light: Color32,
    /// The splash's title, its small print and its build line.
    pub(crate) splash: [Color32; 3],
    lanes: [Color32; 10],
}

pub(crate) const BLOOD_MOON: Theme = Theme {
    name: "blood-moon",
    dark: true,
    bg: Color32::from_rgb(23, 7, 9),
    panel: Color32::from_rgb(38, 14, 18),
    panel_raised: Color32::from_rgb(48, 18, 23),
    rule: Color32::from_rgb(75, 34, 40),
    dim: Color32::from_rgb(140, 91, 98),
    accent: Color32::from_rgb(230, 57, 53),
    text: Color32::from_rgb(233, 213, 211),
    error_text: Color32::from_rgb(255, 138, 122),
    warn: Color32::from_rgb(210, 153, 34),
    warn_text: Color32::from_rgb(242, 204, 96),
    debug: Color32::from_rgb(57, 197, 207),
    trace: Color32::from_rgb(188, 140, 255),
    online: Color32::from_rgb(63, 185, 80),
    deep: Color32::from_rgb(16, 5, 7),
    selection: Color32::from_rgb(120, 30, 34),
    hover: Color32::from_rgb(70, 26, 31),
    light: Color32::WHITE,
    splash: [Color32::from_rgb(255, 215, 208), Color32::from_rgb(229, 115, 115), Color32::from_rgb(255, 171, 145)],
    lanes: [
        Color32::from_rgb(255, 83, 112),
        Color32::from_rgb(255, 148, 82),
        Color32::from_rgb(255, 214, 102),
        Color32::from_rgb(63, 200, 120),
        Color32::from_rgb(57, 197, 178),
        Color32::from_rgb(87, 214, 224),
        Color32::from_rgb(100, 148, 255),
        Color32::from_rgb(163, 133, 255),
        Color32::from_rgb(255, 121, 198),
        Color32::from_rgb(255, 110, 149),
    ],
};

/// A light morning palette: warm paper, the same red accent, the lanes and the log colors
/// deepened so they read on white.
pub(crate) const DAYBREAK: Theme = Theme {
    name: "daybreak",
    dark: false,
    bg: Color32::from_rgb(250, 246, 243),
    panel: Color32::from_rgb(242, 235, 231),
    panel_raised: Color32::from_rgb(255, 255, 255),
    rule: Color32::from_rgb(214, 200, 196),
    dim: Color32::from_rgb(138, 112, 116),
    accent: Color32::from_rgb(200, 40, 46),
    text: Color32::from_rgb(44, 24, 28),
    error_text: Color32::from_rgb(190, 40, 30),
    warn: Color32::from_rgb(196, 128, 10),
    warn_text: Color32::from_rgb(160, 104, 0),
    debug: Color32::from_rgb(0, 130, 142),
    trace: Color32::from_rgb(112, 70, 200),
    online: Color32::from_rgb(28, 140, 52),
    deep: Color32::from_rgb(255, 252, 250),
    selection: Color32::from_rgb(250, 208, 204),
    hover: Color32::from_rgb(244, 224, 220),
    light: Color32::BLACK,
    splash: [Color32::from_rgb(60, 20, 24), Color32::from_rgb(170, 60, 62), Color32::from_rgb(120, 70, 66)],
    lanes: [
        Color32::from_rgb(214, 44, 78),
        Color32::from_rgb(214, 104, 30),
        Color32::from_rgb(180, 140, 0),
        Color32::from_rgb(28, 150, 80),
        Color32::from_rgb(18, 150, 134),
        Color32::from_rgb(20, 150, 170),
        Color32::from_rgb(50, 100, 220),
        Color32::from_rgb(120, 90, 220),
        Color32::from_rgb(200, 60, 150),
        Color32::from_rgb(210, 60, 100),
    ],
};

/// Every theme, in the order `theme` lists them; the first is the default.
pub(crate) const ALL: [Theme; 2] = [BLOOD_MOON, DAYBREAK];

/// The index into `ALL` of the current theme.
static CURRENT: AtomicUsize = AtomicUsize::new(0);

/// The theme the window paints with now.
pub(crate) fn current() -> Theme {
    ALL[CURRENT.load(Ordering::Relaxed) % ALL.len()]
}

/// The theme called `name`, matched loosely: `daybreak`, `Daybreak`, `blood moon` all find one.
pub(crate) fn by_name(name: &str) -> Option<Theme> {
    let wanted: String = name.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase();
    ALL.into_iter().find(|t| t.name.replace('-', "") == wanted)
}

/// Makes `theme` current; the caller sets the visuals on the context so the widgets follow.
pub(crate) fn set(theme: Theme) {
    if let Some(i) = ALL.iter().position(|t| t.name == theme.name) {
        CURRENT.store(i, Ordering::Relaxed);
    }
}

pub(crate) fn bg() -> Color32 {
    current().bg
}
pub(crate) fn panel() -> Color32 {
    current().panel
}
pub(crate) fn panel_raised() -> Color32 {
    current().panel_raised
}
pub(crate) fn rule() -> Color32 {
    current().rule
}
pub(crate) fn dim() -> Color32 {
    current().dim
}
pub(crate) fn accent() -> Color32 {
    current().accent
}
pub(crate) fn text() -> Color32 {
    current().text
}
pub(crate) fn error_text() -> Color32 {
    current().error_text
}
pub(crate) fn warn() -> Color32 {
    current().warn
}
pub(crate) fn warn_text() -> Color32 {
    current().warn_text
}
pub(crate) fn debug() -> Color32 {
    current().debug
}
pub(crate) fn trace() -> Color32 {
    current().trace
}
pub(crate) fn online() -> Color32 {
    current().online
}
pub(crate) fn deep() -> Color32 {
    current().deep
}
pub(crate) fn light() -> Color32 {
    current().light
}

/// The bright lane color of a depth.
pub(crate) fn lane(depth: usize) -> Color32 {
    let lanes = &current().lanes;
    lanes[depth % lanes.len()]
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
    let t = current();
    match state {
        S::Defined => t.dim,
        S::Ready | S::Assigned => t.trace,
        S::InProgress | S::Testing | S::Review => t.warn_text,
        S::Implemented => t.debug,
        S::Accepted => t.online,
        S::Blocked | S::NeedsRevision => t.warn,
        S::Failed | S::Cancelled => t.error_text,
    }
}

/// The visuals of the current theme: its surfaces, the red accent, soft corners.
pub(crate) fn visuals() -> Visuals {
    let t = current();
    let mut v = if t.dark { Visuals::dark() } else { Visuals::light() };
    v.override_text_color = Some(t.text);
    v.panel_fill = t.bg;
    v.window_fill = t.panel;
    v.extreme_bg_color = t.deep;
    v.faint_bg_color = t.panel;
    v.window_stroke = Stroke::new(1.0, t.rule);
    v.selection.bg_fill = t.selection;
    v.selection.stroke = Stroke::new(1.0, t.accent);
    v.hyperlink_color = t.accent;
    v.warn_fg_color = t.warn_text;
    v.error_fg_color = t.error_text;
    let widgets = &mut v.widgets;
    widgets.noninteractive.bg_fill = t.panel;
    widgets.noninteractive.weak_bg_fill = t.panel;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, t.rule);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, t.dim);
    widgets.inactive.bg_fill = t.panel_raised;
    widgets.inactive.weak_bg_fill = t.panel_raised;
    widgets.inactive.bg_stroke = Stroke::new(1.0, t.rule);
    widgets.inactive.fg_stroke = Stroke::new(1.0, t.text);
    widgets.hovered.bg_fill = t.hover;
    widgets.hovered.weak_bg_fill = t.hover;
    widgets.hovered.bg_stroke = Stroke::new(1.0, t.accent);
    widgets.hovered.fg_stroke = Stroke::new(1.5, t.text);
    widgets.active.bg_fill = t.selection;
    widgets.active.bg_stroke = Stroke::new(1.0, t.accent);
    widgets.active.fg_stroke = Stroke::new(2.0, t.text);
    widgets.open.bg_fill = t.panel_raised;
    widgets.open.bg_stroke = Stroke::new(1.0, t.rule);
    for w in [&mut widgets.noninteractive, &mut widgets.inactive, &mut widgets.hovered, &mut widgets.active, &mut widgets.open] {
        w.corner_radius = CornerRadius::same(6);
    }
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v
}
