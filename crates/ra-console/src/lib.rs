//! The Rhipper Cmd console: a full-screen terminal UI over the orchestrator, in the Blood Moon
//! theme of the workbench. A status line on top, the session log in the middle, and a prompt at
//! the bottom. Only the binary uses it; anything not on a terminal logs plainly instead.

pub mod logs;
pub mod theme;
