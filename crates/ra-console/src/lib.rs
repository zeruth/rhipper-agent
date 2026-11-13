//! The Rhipper Agent console: a full-screen terminal UI over the roadmap and the workers, in the
//! Blood Moon theme of the workbench. A status line on top, the session log in the middle, and a
//! prompt at the bottom for the [`commands`]. Only the binary uses it; anything not on a terminal
//! logs plainly instead.

pub mod agent;
pub mod allocation;
pub mod allow_exe;
pub mod allow_move;
pub mod channel;
pub mod clock;
pub mod commands;
pub mod gotcha_shell_variable;
pub mod hub;
pub mod install;
pub mod instance;
pub mod logs;
pub mod lookback;
pub mod mcp;
pub mod pair;
pub mod plan;
pub mod retro;
pub mod review;
pub mod roadmap_pane;
pub mod roles;
pub mod router;
pub mod secret;
pub mod setup;
pub mod theme;
pub mod trace;
pub mod ui;
pub mod walk;

/// The version, `0.2.<n>`: `n` the commits of the tree the build is of, `+` after it when
/// the sources had changes past that commit.
pub const VERSION: &str = env!("RA_VERSION");
