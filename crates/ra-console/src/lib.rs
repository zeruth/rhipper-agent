//! The Rhipper Agent console: a full-screen terminal UI over the roadmap and the workers, in the
//! Blood Moon theme of the workbench. A status line on top, the session log in the middle, and a
//! prompt at the bottom for the [`commands`]. Only the binary uses it; anything not on a terminal
//! logs plainly instead.

pub mod agent;
pub mod commands;
pub mod logs;
pub mod roadmap_pane;
pub mod theme;
pub mod ui;
