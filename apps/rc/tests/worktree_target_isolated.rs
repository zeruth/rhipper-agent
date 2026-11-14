//! A run in a worktree builds in a target of its own, seeded from the main checkout's, so its
//! first build is warm and its check never links a crate another tree's sources built.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use ra_providers::claude_code::{ClaudeCode, project_target, seed_target, shared_target};

fn write(path: &Path, text: &str, modified: SystemTime) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
    std::fs::File::options().write(true).open(path).unwrap().set_modified(modified).unwrap();
}

fn modified(path: &Path) -> SystemTime {
    std::fs::metadata(path).unwrap().modified().unwrap()
}

#[test]
fn a_worktree_builds_in_its_own_target_under_the_projects() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let worktree = dir.path().join("var").join("worktrees").join("663");
    let own = std::path::absolute(project_target(dir.path()).join("wt-663")).expect("an absolute path");

    assert_eq!(shared_target(&worktree), Some(own.clone()));
    let env = ClaudeCode::new().run_env(Some(&worktree));
    let target = env.iter().find(|(k, _)| k == "CARGO_TARGET_DIR").map(|(_, v)| PathBuf::from(v));
    assert_eq!(target, Some(own));
}

#[test]
fn two_worktrees_never_share_a_target() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let worktrees = dir.path().join("var").join("worktrees");
    let main = std::path::absolute(project_target(dir.path())).unwrap();
    let (one, two) = (shared_target(&worktrees.join("1")).unwrap(), shared_target(&worktrees.join("2")).unwrap());
    assert_ne!(one, two);
    assert_ne!(one, main);
    assert_ne!(two, main);
}

#[test]
fn a_seeded_target_is_a_dated_copy_the_other_tree_cannot_touch() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let main = dir.path().join("target");
    let own = main.join("wt-7");
    let then = SystemTime::now() - Duration::from_secs(3600);
    write(&main.join("debug").join(".cargo-lock"), "", then);
    write(&main.join("debug").join("deps").join("liblift.rlib"), "main's lift", then);
    write(&main.join("debug").join(".fingerprint").join("lift-1").join("lib-lift"), "hash", then);
    write(&main.join("debug").join("incremental").join("lift-1").join("s"), "session", then);
    write(&main.join(".rustc_info.json"), "{}", then);
    write(&main.join("wt-3").join("debug").join("deps").join("liblift.rlib"), "another tree's lift", then);

    assert!(seed_target(&main, &own).unwrap() > 0);
    let rlib = own.join("debug").join("deps").join("liblift.rlib");
    assert_eq!(std::fs::read_to_string(&rlib).unwrap(), "main's lift");
    assert_eq!(modified(&rlib), modified(&main.join("debug").join("deps").join("liblift.rlib")));
    assert!(own.join("debug").join(".fingerprint").join("lift-1").join("lib-lift").is_file());
    assert!(own.join(".rustc_info.json").is_file());
    assert!(!own.join("debug").join("incremental").exists(), "the incremental sessions are not seeded");
    assert!(!own.join("wt-3").exists(), "another worktree's target is not seeded");
    assert!(!main.join("wt-7.seeding").exists());

    // a build in the worktree rewrites its own copy, never the main checkout's file
    std::fs::write(&rlib, "the worktree's lift").unwrap();
    assert_eq!(std::fs::read_to_string(main.join("debug").join("deps").join("liblift.rlib")).unwrap(), "main's lift");

    // a target that is there is left as it is
    assert_eq!(seed_target(&main, &own).unwrap(), 0);
    assert_eq!(std::fs::read_to_string(&rlib).unwrap(), "the worktree's lift");
}
