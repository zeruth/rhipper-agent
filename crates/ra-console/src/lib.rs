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
pub mod gotcha_edit_space;
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
pub mod route;
pub mod router;
pub mod secret;
pub mod setup;
pub mod theme;
pub mod trace;
pub mod ui;
pub mod walk;

/// What answers a cargo command in a test whose worker scripts no cargo: a panic, since a real
/// build runs for the length of a suite and makes the tests that wait on it look hung.
#[cfg(test)]
pub(crate) fn unscripted(command: &str) -> anyhow::Result<(i32, String)> {
    panic!("unscripted cargo in a test: {command}")
}

/// The feed's `wait:` line of a check that `who` (a [`hub::builder_name`]) runs: `text` with the
/// check's builder after it, so a timed-out check names its own waits, not another card's.
#[must_use]
pub fn wait_line(text: &str, who: &str) -> String {
    format!("{text} · for {who}")
}

/// [`timeout_note_with_waits`] of a check run in `acting`, a card's worktree or the project,
/// with the paired feed of its project and its builder's name.
#[must_use]
pub fn timeout_note(acting: &std::path::Path, output: &str) -> String {
    let feed = pair::recent(&hub::project_root_of(acting), 50);
    timeout_note_with_waits(output, &feed, &hub::builder_name(acting))
}

/// A timed-out check's output with what its hung tests waited on: one line for each test the
/// output says has been running for over so long, naming the latest `wait:` line of the `feed`
/// that the check of `who` said (a lock such as the cargo target's, or a build slot). Output that
/// did not time out, and a feed without a wait of that check, come back as they were.
#[must_use]
pub fn timeout_note_with_waits(output: &str, feed: &[String], who: &str) -> String {
    let own = wait_line("", who);
    let Some(wait) = feed.iter().rev().find(|l| l.starts_with("wait:") && l.ends_with(&own)) else { return output.to_owned() };
    if !output.lines().any(|l| l.starts_with("timed out after ")) {
        return output.to_owned();
    }
    let hung: Vec<&str> = output
        .lines()
        .filter(|l| l.contains("has been running for over"))
        .filter_map(|l| l.trim().strip_prefix("test ")?.split_whitespace().next())
        .collect();
    let mut text = output.trim_end().to_owned();
    for name in hung {
        text.push('\n');
        text.push_str(name);
        text.push(' ');
        text.push_str(wait);
    }
    text
}

/// The version, `0.2.<n>`: `n` the commits of the tree the build is of, `+` after it when
/// the sources had changes past that commit.
pub const VERSION: &str = env!("RA_VERSION");
