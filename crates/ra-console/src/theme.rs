//! The console's colors, from the Blood Moon theme of the workbench
//! (`plugins/rhipper/core/resources/themes/BloodMoon.theme.json`). The surfaces stay dark; only
//! the text and the outlines carry the bright material colors.

use ratatui::style::Color;

/// The console's background: near-black maroon, gray-10 of the Blood Moon ramp.
pub const BG: Color = Color::Rgb(23, 7, 9); // gray-10 #170709
/// The raised surfaces: the header, the usage pane and the prompt box.
pub const PANEL: Color = Color::Rgb(38, 14, 18); // between gray-10 and gray-30, a step above BG
/// The outline of the panels and the separators.
pub const RULE: Color = Color::Rgb(75, 34, 40); // gray-60 #4B2228
/// The muted text: the hints, the timestamps, the separators.
pub const DIM: Color = Color::Rgb(140, 91, 98); // gray-90 #8C5B62
/// The accent: the theme's red, for the keys and the highlights only.
pub const ACCENT: Color = Color::Rgb(230, 57, 53); // blue-90 #E53935
pub const ACCENT_DEEP: Color = Color::Rgb(198, 40, 40); // blue-80 #C62828
/// The regular text.
pub const TEXT: Color = Color::Rgb(233, 213, 211); // editor-text #E9D5D3
/// The command output.
pub const OUTPUT: Color = Color::Rgb(255, 111, 96); // blue-100 #FF6F60
pub const ERROR: Color = Color::Rgb(198, 40, 40); // blue-80 #C62828
pub const ERROR_TEXT: Color = Color::Rgb(255, 138, 122); // blue-110 #FF8A7A
pub const WARN: Color = Color::Rgb(210, 153, 34);
pub const WARN_TEXT: Color = Color::Rgb(242, 204, 96);
pub const DEBUG: Color = Color::Rgb(57, 197, 207);
pub const TRACE: Color = Color::Rgb(188, 140, 255);
pub const ONLINE: Color = Color::Rgb(63, 185, 80);

/// The lane colors of the tree depth, GitKraken style: one vivid color per level of the roadmap.
const LANES: [(u8, u8, u8); 10] = [
    (255, 83, 112),  // red
    (255, 148, 82),  // orange
    (255, 214, 102), // yellow
    (63, 200, 120),  // green
    (57, 197, 178),  // teal
    (87, 214, 224),  // cyan
    (100, 148, 255), // blue
    (163, 133, 255), // violet
    (255, 121, 198), // magenta
    (255, 110, 149), // rose
];

/// The bright lane color of a depth.
pub fn lane(depth: usize) -> Color {
    let (r, g, b) = LANES[depth % LANES.len()];
    Color::Rgb(r, g, b)
}

/// The lane color of a depth, dimmed toward the background for the cards out of the hover chain.
pub fn dim_lane(depth: usize) -> Color {
    let (r, g, b) = LANES[depth % LANES.len()];
    let mix = |c: u8| u8::try_from(u16::from(c) * 45 / 100 + 12).unwrap_or(c);
    Color::Rgb(mix(r), mix(g), mix(b))
}
