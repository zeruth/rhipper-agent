//! `sha256sum` in a run's shell: the hook allows it on paths inside the worktree.

use std::path::Path;

use ra_gotchas::shape::{Shape, Shell, judge};

fn judged(command: &str) -> Shape {
    judge(command, Shell::Bash, Path::new("."), &|_| false)
}

#[test]
fn sha256sum_inside_the_worktree_is_allowed() {
    assert_eq!(judged("sha256sum target/out.bin"), Shape::Allowed);
    assert_eq!(judged("sha256sum a.bin b.bin | sort"), Shape::Allowed);
}

#[test]
fn sha256sum_out_of_the_worktree_is_not_allowed() {
    for command in ["sha256sum ../a.bin", "sha256sum /etc/passwd"] {
        assert_ne!(judged(command), Shape::Allowed, "{command}");
    }
}
