//! The console's colors, from the Blood Moon theme of the workbench
//! (`plugins/rhipper/core/resources/themes/BloodMoon.theme.json`).

use ratatui::style::Color;

/// The console's background, and the header's and the prompt's.
pub const BG: Color = Color::Rgb(23, 7, 9); // gray-10 #170709
pub const PANEL: Color = Color::Rgb(38, 14, 18); // gray-30 #260E12
/// The accent: the theme's red.
pub const ACCENT: Color = Color::Rgb(230, 57, 53); // blue-90 #E53935
pub const ACCENT_DEEP: Color = Color::Rgb(198, 40, 40); // blue-80 #C62828
pub const RULE: Color = Color::Rgb(75, 34, 40); // gray-60 #4B2228
pub const DIM: Color = Color::Rgb(140, 91, 98); // gray-90 #8C5B62
pub const TEXT: Color = Color::Rgb(233, 213, 211); // editor-text #E9D5D3
pub const OUTPUT: Color = Color::Rgb(255, 111, 96); // blue-100 #FF6F60
pub const ERROR: Color = Color::Rgb(198, 40, 40); // blue-80 #C62828
pub const ERROR_TEXT: Color = Color::Rgb(255, 138, 122); // blue-110 #FF8A7A
pub const WARN: Color = Color::Rgb(210, 153, 34);
pub const WARN_TEXT: Color = Color::Rgb(242, 204, 96);
pub const DEBUG: Color = Color::Rgb(57, 197, 207);
pub const TRACE: Color = Color::Rgb(188, 140, 255);
pub const ONLINE: Color = Color::Rgb(63, 185, 80);
