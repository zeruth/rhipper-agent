//! A run in a worktree builds in the main checkout's target dir, where the project's binaries
//! are already built.

use ra_providers::claude_code::{ClaudeCode, project_target, shared_target};

#[test]
fn a_worktree_builds_in_the_main_checkouts_target_dir() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let worktree = dir.path().join("var").join("worktrees").join("663");
    let main = std::path::absolute(project_target(dir.path())).expect("an absolute path");

    assert_eq!(shared_target(&worktree), Some(main.clone()));
    let env = ClaudeCode::new().run_env(Some(&worktree));
    let target = env.iter().find(|(k, _)| k == "CARGO_TARGET_DIR").map(|(_, v)| std::path::PathBuf::from(v));
    assert_eq!(target, Some(main));
}

#[test]
fn two_worktrees_share_one_target_dir() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let worktrees = dir.path().join("var").join("worktrees");
    assert_eq!(shared_target(&worktrees.join("1")), shared_target(&worktrees.join("2")));
}
