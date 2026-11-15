//! A check does not run a test binary built before the library it links: the binary and its
//! fingerprint go, so cargo rebuilds it, and the output says so.

use std::path::Path;
use std::time::{Duration, SystemTime};

use ra_providers::rebuild::rebuild_stale_binaries;

fn touch(path: &Path, age: u64) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "x").unwrap();
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(age)).unwrap();
}

#[test]
fn a_test_binary_older_than_its_library_is_rebuilt_not_run() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let target = dir.path();
    let deps = target.join("debug").join("deps");
    let prints = target.join("debug").join(".fingerprint");
    let print = prints.join("lift-0123456789abcdef");
    touch(&print.join("test-lib-lift"), 100);
    touch(&deps.join("liblift-fedcba9876543210.rlib"), 10);
    touch(&deps.join("lift-0123456789abcdef.exe"), 100);
    touch(&deps.join("lift-0123456789abcdef"), 100);
    // an integration test of the package, named by its package with a dash, linking the lib
    let uses = prints.join("uses-lift-1111111111111111");
    std::fs::create_dir_all(&uses).unwrap();
    std::fs::write(uses.join("test-integration-test-uses_lift.json"), r#"{"deps":[[1,"lift",false,2],[3,"anyhow",false,4]]}"#).unwrap();
    touch(&deps.join("uses_lift-1111111111111111.exe"), 100);
    // an old test binary of another crate that does not link the lib
    let other = prints.join("other-2222222222222222");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("test-integration-test-other.json"), r#"{"deps":[[3,"anyhow",false,4]]}"#).unwrap();
    touch(&deps.join("other-2222222222222222.exe"), 100);
    // an old artifact that is no test binary
    touch(&prints.join("build-3333333333333333").join("bin-build"), 100);
    touch(&deps.join("build-3333333333333333"), 100);
    touch(&prints.join("fresh-0123456789abcdee").join("test-lib-fresh"), 1);
    touch(&deps.join("fresh-0123456789abcdee.exe"), 1);

    let crates = ["lift".to_owned()];
    let mut lines = rebuild_stale_binaries(target, &crates).unwrap();
    lines.sort();
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(lines.iter().all(|l| l.contains("rebuilt: stale against liblift-fedcba9876543210.rlib")), "{lines:?}");
    assert!(lines[2].starts_with("uses_lift-1111111111111111.exe "), "{lines:?}");
    assert!(!deps.join("lift-0123456789abcdef.exe").exists());
    assert!(!deps.join("lift-0123456789abcdef").exists());
    assert!(!deps.join("uses_lift-1111111111111111.exe").exists());
    assert!(!print.exists(), "the fingerprint goes with the binary");
    assert!(!uses.exists(), "the package's fingerprint goes with its crate's binary");
    assert!(deps.join("other-2222222222222222.exe").exists(), "a binary that does not link the library stays");
    assert!(other.exists());
    assert!(deps.join("build-3333333333333333").exists(), "an artifact that is no test binary stays");
    assert!(deps.join("fresh-0123456789abcdee.exe").exists(), "a binary newer than the library stays");
    assert!(deps.join("liblift-fedcba9876543210.rlib").exists());

    assert!(rebuild_stale_binaries(target, &crates).unwrap().is_empty(), "nothing stale the second time");
    assert!(rebuild_stale_binaries(target, &["other".to_owned()]).unwrap().is_empty());
}
