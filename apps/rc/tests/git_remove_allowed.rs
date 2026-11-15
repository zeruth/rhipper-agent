//! `git rm` in a run's shell: the hook allows it for tracked paths inside the worktree.

use std::path::Path;
use std::process::Command;

use ra_gotchas::shape::{Shape, Shell, judge};

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git").args(args).current_dir(dir).status().unwrap();
    assert!(status.success(), "git {args:?}");
}

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    std::fs::create_dir_all(dir.path().join("tests")).unwrap();
    std::fs::write(dir.path().join("tests/a.rs"), "a").unwrap();
    std::fs::write(dir.path().join("tests/b.rs"), "b").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "x"]);
    std::fs::write(dir.path().join("loose.rs"), "c").unwrap();
    dir
}

fn judged(dir: &Path, command: &str) -> Shape {
    judge(command, Shell::Bash, dir, &|_| false)
}

#[test]
fn git_rm_of_tracked_paths_inside_the_worktree_is_allowed() {
    let dir = repo();
    for command in ["git rm tests/a.rs", "git rm -r tests", "git rm -f tests/a.rs tests/b.rs", "git rm -- tests/a.rs"] {
        assert_eq!(judged(dir.path(), command), Shape::Allowed, "{command}");
    }
}

#[test]
fn git_rm_of_an_outside_or_untracked_path_is_refused() {
    let dir = repo();
    for command in ["git rm ../a.rs", "git rm /etc/hosts", "git rm loose.rs", "git rm tests/a.rs loose.rs"] {
        assert!(matches!(judged(dir.path(), command), Shape::Denied(_)), "{command}");
    }
}

#[test]
fn a_forced_git_rm_over_uncommitted_changes_is_refused() {
    let dir = repo();
    std::fs::write(dir.path().join("tests/a.rs"), "changed").unwrap();
    assert!(matches!(judged(dir.path(), "git rm -f tests/a.rs"), Shape::Denied(_)));
}
