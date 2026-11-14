//! A refused `ls` of a path outside the worktree names its gotcha, and the gotcha says where the
//! worker may look instead.

use ra_gotchas::{Catalogue, Conditions, Verdict};

#[test]
fn an_ls_outside_the_worktree_matches_its_gotcha() {
    let catalogue = Catalogue::seed();
    for command in ["ls ../../target/debug/ras.exe", "ls C:/Windows/SysWOW64/msvcrt.dll", "ls -la ../../target/debug"] {
        let id = catalogue.matching(command).map(|g| g.id.as_str());
        assert_eq!(id, Some("ls-outside-worktree"), "{command}");
    }
}

#[test]
fn the_gotcha_names_the_worktrees_target_and_the_system_paths() {
    let catalogue = Catalogue::seed();
    let Verdict::Deny { id, reason } = catalogue.command("ls ../../target/debug/ras.exe", Conditions::default()) else {
        panic!("the ls is refused with a hint");
    };
    assert_eq!(id, "ls-outside-worktree");
    assert!(reason.contains("target/"), "{reason}");
    assert!(reason.contains("system paths"), "{reason}");
}

#[test]
fn an_ls_inside_the_worktree_is_not_named() {
    let catalogue = Catalogue::seed();
    for command in ["ls target/debug", "ls crates/ra-gotchas/src", "ls -la apps/rc/tests"] {
        let id = catalogue.matching(command).map(|g| g.id.as_str());
        assert_ne!(id, Some("ls-outside-worktree"), "{command}");
    }
}
