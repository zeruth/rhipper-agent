use std::path::Path;

use ra_console::allow_move::allows_move;

fn worktree() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn a_move_inside_the_worktree_is_allowed() {
    assert!(allows_move("git mv src/a.rs src/b.rs", worktree()));
    assert!(allows_move("git mv -k src/a.rs src/sub/a.rs", worktree()));
    let root = worktree().display().to_string();
    assert!(allows_move(&format!("git mv {root}/src/a.rs {root}/src/b.rs"), worktree()));
}

#[test]
fn a_move_with_a_path_outside_the_worktree_is_refused() {
    assert!(!allows_move("git mv src/a.rs ../b.rs", worktree()));
    assert!(!allows_move("git mv ../a.rs src/b.rs", worktree()));
    assert!(!allows_move("git mv src/a.rs /etc/b.rs", worktree()));
    assert!(!allows_move("git mv src/a.rs C:/elsewhere/b.rs", worktree()));
}

#[test]
fn a_move_without_a_destination_is_not_allowed() {
    assert!(!allows_move("git mv src/a.rs", worktree()));
}
