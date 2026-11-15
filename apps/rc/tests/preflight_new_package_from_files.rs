//! A card whose files lie under a package directory that does not exist yet creates the package:
//! the preflight does not reject its `-p` check.

use ra_console::agent::names_new_package;

fn files() -> Vec<String> {
    vec!["crates/ra-asm-sh/src/decode.rs".to_owned(), "crates/ra-asm-sh/src/lib.rs".to_owned()]
}

#[test]
fn files_under_a_missing_directory_make_the_package_new() {
    let dir = tempfile::tempdir().expect("a tempdir");
    assert!(names_new_package(dir.path(), "cargo test -p ra-asm-sh", &files()));
    assert!(names_new_package(dir.path(), "cargo test --package=ra-asm-sh -q", &files()));
}

#[test]
fn an_existing_directory_or_another_package_is_not_new() {
    let dir = tempfile::tempdir().expect("a tempdir");
    std::fs::create_dir_all(dir.path().join("crates/ra-asm-sh")).expect("the crate dir");
    assert!(!names_new_package(dir.path(), "cargo test -p ra-asm-sh", &files()));
    assert!(!names_new_package(dir.path(), "cargo test -p ra-other", &files()));
    assert!(!names_new_package(dir.path(), "python -p ra-asm-sh", &files()));
}
