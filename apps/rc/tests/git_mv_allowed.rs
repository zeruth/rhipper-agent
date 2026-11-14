//! `git mv` in a run's shell: the hook allows it inside the worktree and refuses it outside.

use std::path::Path;

use ra_gotchas::shape::{Shape, Shell, judge};

fn judged(command: &str) -> Shape {
    judge(command, Shell::Bash, Path::new("."), &|_| false)
}

#[test]
fn git_mv_inside_the_worktree_is_allowed() {
    assert_eq!(judged("git mv src/a.rs src/b.rs"), Shape::Allowed);
}

#[test]
fn git_mv_out_of_the_worktree_is_refused() {
    for command in ["git mv src/a.rs ../b.rs", "git mv ../a.rs src/b.rs", "git mv a.rs /etc/b.rs"] {
        assert!(matches!(judged(command), Shape::Denied(_)), "{command}");
    }
}

#[test]
fn git_mv_without_a_destination_is_not_allowed() {
    assert_ne!(judged("git mv src/a.rs"), Shape::Allowed);
}
