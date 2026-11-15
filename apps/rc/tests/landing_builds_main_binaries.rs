//! A landing in a project whose builds are refused builds the workspace's binaries into the
//! project's main target, where the next worktree's seed finds them.

use ra_console::agent::card_tree;
use ra_providers::claude_code::{build_binaries, seed_binaries};

fn project(dir: &std::path::Path, body: &str) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"tool\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    std::fs::write(dir.join("src").join("main.rs"), body).unwrap();
}

#[test]
fn the_binaries_are_built_into_the_main_target_and_seeded_from_it() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let (root, main, own) = (dir.path().join("root"), dir.path().join("target"), dir.path().join("target").join("wt-9"));
    project(&root, "fn main() {}\n");

    build_binaries(&root, &main).expect("the build passes");
    let copied = seed_binaries(&main, &own).unwrap();
    assert_eq!(copied.len(), 1, "{copied:?}");
    assert!(copied[0].file_name().unwrap().to_string_lossy().starts_with("tool"));
}

#[test]
fn a_worktree_seeded_with_an_older_binary_gets_one_built_into_main_later() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let (main, own) = (dir.path().join("target"), dir.path().join("target").join("wt-238"));
    std::fs::create_dir_all(main.join("debug")).unwrap();
    program(&main.join("debug").join(exe("rc")), "rc");
    assert_eq!(seed_binaries(&main, &own).unwrap(), vec![own.join("debug").join(exe("rc"))]);

    program(&main.join("debug").join(exe("ras")), "ras");
    let copied = seed_binaries(&main, &own).unwrap();
    assert_eq!(copied, vec![own.join("debug").join(exe("ras")), own.join("debug").join(exe("rc"))]);
    assert_eq!(std::fs::read_to_string(own.join("debug").join(exe("ras"))).unwrap(), "ras");
}

#[test]
fn a_run_seeds_the_cards_worktree_not_the_project() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let root = dir.path();
    assert_eq!(card_tree(root, 238), root, "a card without a worktree acts in the project");

    let worktree = root.join("var").join("worktrees").join("238");
    std::fs::create_dir_all(&worktree).unwrap();
    std::fs::write(worktree.join(".git"), "gitdir: elsewhere").unwrap();
    assert_eq!(card_tree(root, 238), worktree);
}

/// The file name of a program on this platform; the seed copies only those.
fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

/// Writes a program at `path`: the text, with the execute bit where there is one.
fn program(path: &std::path::Path, text: &str) {
    std::fs::write(path, text).unwrap();
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[test]
fn a_failed_build_is_an_error_to_say_not_a_panic() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let (root, main) = (dir.path().join("root"), dir.path().join("target"));
    project(&root, "fn main() { nope }\n");

    let error = build_binaries(&root, &main).unwrap_err();
    assert!(error.contains("failed"), "{error}");
}
