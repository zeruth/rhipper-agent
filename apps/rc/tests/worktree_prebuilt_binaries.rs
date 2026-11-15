//! A worktree in a project whose builds are refused gets the main checkout's built binaries in
//! its own target, and the handoff names that path; with none built it says so.

use ra_providers::claude_code::{prebuilt_note, seed_binaries};

fn exe(name: &str) -> String {
    if cfg!(windows) { format!("{name}.exe") } else { name.to_owned() }
}

fn program(path: &std::path::Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "binary").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[test]
fn the_built_executables_are_copied_and_named_in_the_handoff() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let (main, own) = (dir.path().join("target"), dir.path().join("target").join("wt-9"));
    program(&main.join("debug").join(exe("gen")));
    std::fs::create_dir_all(main.join("debug").join("deps")).unwrap();
    std::fs::write(main.join("debug").join("deps").join("libx.rlib"), "lib").unwrap();
    std::fs::write(main.join("debug").join("gen.d"), "dep").unwrap();

    let note = prebuilt_note(&main, &own);
    let copy = own.join("debug").join(exe("gen"));
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), "binary");
    assert!(note.contains(&own.join("debug").display().to_string()), "{note}");
    assert!(note.contains(&exe("gen")), "{note}");
    assert!(!own.join("debug").join("deps").exists(), "only the executables are copied");
    assert!(!own.join("debug").join("gen.d").exists());
    assert_eq!(seed_binaries(&main, &own).unwrap(), vec![copy]);
}

#[test]
fn with_no_built_binary_the_handoff_says_to_hand_write_the_generated_files() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let (main, own) = (dir.path().join("target"), dir.path().join("target").join("wt-9"));

    assert!(seed_binaries(&main, &own).unwrap().is_empty());
    let note = prebuilt_note(&main, &own);
    assert!(note.contains("no built binary"), "{note}");
    assert!(note.contains("hand-written"), "{note}");
    assert!(!own.exists());
}
