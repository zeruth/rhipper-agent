//! A card builds in `<project target>/wt-<card id>`, never a pooled `wt-pool-<n>` another card
//! built in before it.

use ra_providers::claude_code::{project_target, seed_worktree_target, shared_target};

#[test]
fn a_card_builds_in_a_target_named_for_its_id() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let worktree = dir.path().join("var").join("worktrees").join("796");
    seed_worktree_target(&worktree).expect("seeding a worktree with no main target");
    let own = std::path::absolute(project_target(dir.path()).join("wt-796")).expect("an absolute path");
    assert_eq!(shared_target(&worktree), Some(own));
}

#[test]
fn two_cards_never_share_a_target() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let worktrees = dir.path().join("var").join("worktrees");
    let (one, two) = (worktrees.join("1"), worktrees.join("2"));
    seed_worktree_target(&one).unwrap();
    seed_worktree_target(&two).unwrap();
    let (a, b) = (shared_target(&one).unwrap(), shared_target(&two).unwrap());
    assert_ne!(a, b);
    assert!(!a.to_string_lossy().contains("wt-pool-") && !b.to_string_lossy().contains("wt-pool-"));
}
