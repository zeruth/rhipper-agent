use super::*;

#[test]
fn install_copies_the_build_it_made() {
    let suffix = std::env::consts::EXE_SUFFIX;
    let rc = format!("rc{suffix}");
    let gui = format!("rc-gui{suffix}");
    // a stale build in the checkout's own target, the fresh one under CARGO_TARGET_DIR
    let root = tempfile::tempdir().unwrap();
    let stale = root.path().join("target").join("debug");
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join(&rc), "0.2.484").unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let fresh = elsewhere.path().join("debug");
    std::fs::create_dir_all(&fresh).unwrap();
    std::fs::write(fresh.join(&rc), "0.2.497").unwrap();
    std::fs::write(fresh.join(&gui), "window").unwrap();
    assert_eq!(build_source(&target_dir(root.path(), None)), Some(stale));
    let source = build_source(&target_dir(root.path(), Some(elsewhere.path().as_os_str()))).unwrap();
    assert_eq!(source, fresh);
    // cargo names the directory it builds this crate into, the one this test runs from
    let built = cargo_target_dir(Path::new(env!("CARGO_MANIFEST_DIR"))).expect("cargo names the target");
    let exe = std::env::current_exe().unwrap();
    assert!(
        exe.starts_with(&built) || std::fs::canonicalize(&exe).unwrap().starts_with(std::fs::canonicalize(&built).unwrap()),
        "{} is not under {}",
        exe.display(),
        built.display()
    );
    // the sources name the version the build script counts
    assert_eq!(sources_version(Path::new(env!("CARGO_MANIFEST_DIR"))).as_deref(), Some(crate::VERSION.trim_end_matches('+')));
    // the version is the file's text here; the window is never asked, it would open
    let read = |p: &Path| {
        assert!(!p.ends_with(&gui), "the window was asked its version: {}", p.display());
        std::fs::read_to_string(p).ok()
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(&rc), "0.2.400").unwrap();
    let lines = copy_binaries(&source, dir.path(), Some("0.2.497"), read).unwrap();
    assert_eq!(std::fs::read_to_string(dir.path().join(&rc)).unwrap(), "0.2.497");
    assert!(lines[0].starts_with(&format!("{rc} 0.2.497 → ")), "{lines:?}");
    assert_eq!(std::fs::read_to_string(dir.path().join(&gui)).unwrap(), "window");
    // the stale build copied while the sources name a newer version is refused, and the
    // installed one comes back
    let other = tempfile::tempdir().unwrap();
    std::fs::write(other.path().join(&rc), "0.2.400").unwrap();
    let stale_source = root.path().join("target").join("debug");
    let refused = copy_binaries(&stale_source, other.path(), Some("0.2.497"), read).unwrap_err().to_string();
    assert!(refused.contains("0.2.497") && refused.contains("0.2.484"), "{refused}");
    assert_eq!(std::fs::read_to_string(other.path().join(&rc)).unwrap(), "0.2.400");
    let left: Vec<_> = std::fs::read_dir(other.path()).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(left, [std::ffi::OsString::from(&rc)]);
    // a version that cannot be read is no refusal
    assert!(check_versions(None, Some("0.2.497")).is_ok());
}

#[test]
fn the_default_dir_is_under_the_user_and_a_sweep_removes_the_set_aside_binaries() {
    let dir = default_dir();
    assert!(dir.ends_with(if cfg!(windows) { "Programs/rhipper" } else { ".local/bin" }), "{}", dir.display());
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("rc.old.1.exe"), b"x").unwrap();
    std::fs::write(tmp.path().join("rc.exe"), b"x").unwrap();
    let mut lines = Vec::new();
    sweep(tmp.path(), &mut lines);
    assert_eq!(lines, ["1 set-aside binary of earlier installs removed"]);
    assert!(tmp.path().join("rc.exe").exists());
    assert!(!tmp.path().join("rc.old.1.exe").exists());
}
