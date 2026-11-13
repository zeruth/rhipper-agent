//! `git mv` in a worker's shell: allowed when both paths resolve inside its worktree, refused
//! when either leaves it. The judgement is the shell table's (`ra_gotchas::shape`); this is the
//! question put to it for a worktree.

use std::path::Path;

use ra_gotchas::shape::{Shape, Shell, judge};

/// Whether the hook lets `command` run in `worktree` as a Bash command: `git mv <src> <dst>`
/// with both paths inside it.
#[must_use]
pub fn allows_move(command: &str, worktree: &Path) -> bool {
    judge(command, Shell::Bash, worktree, &|_| false) == Shape::Allowed
}
