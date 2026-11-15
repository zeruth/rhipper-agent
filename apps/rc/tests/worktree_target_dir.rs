//! A run in a worktree builds in a target of its own, `wt-<id>` under the project's target,
//! never the main checkout's: two trees in one target linked each other's crates (#796).

use ra_providers::claude_code::{ClaudeCode, project_target, shared_target};

#[test]
fn a_worktree_builds_in_its_own_target_under_the_projects() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let worktree = dir.path().join("var").join("worktrees").join("663");
    let main = std::path::absolute(project_target(dir.path())).expect("an absolute path");

    let own = shared_target(&worktree).expect("a worktree has a target");
    assert_eq!(own.file_name().and_then(|n| n.to_str()), Some("wt-663"), "{}", own.display());
    assert_ne!(own, main);
    let env = ClaudeCode::new().run_env(Some(&worktree));
    let target = env.iter().find(|(k, _)| k == "CARGO_TARGET_DIR").map(|(_, v)| std::path::PathBuf::from(v));
    assert_eq!(target, Some(own));
}

#[test]
fn two_worktrees_build_in_two_targets() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let worktrees = dir.path().join("var").join("worktrees");
    assert_ne!(shared_target(&worktrees.join("1")), shared_target(&worktrees.join("2")));
}
