use super::*;

/// A fresh directory under the system's temp, unique to the test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ra-check-ledger-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn row(tree: &str, stamp: &str, outcome: Outcome) -> Row {
    Row {
        at: 1_700_000_000,
        card: "325".to_owned(),
        stage: "worker".to_owned(),
        tree: tree.to_owned(),
        base: "abc123".to_owned(),
        command: "cargo test -p ra-check -q".to_owned(),
        stamp: stamp.to_owned(),
        outcome,
        seconds: 12.5,
        tests: 9,
        blame: String::new(),
    }
}

#[test]
fn a_row_written_is_found_again() {
    let ledger = path(&scratch("found"));
    let written = row("t1", "rustc 1.95.0", Outcome::Passed);
    append(&ledger, &written).unwrap();
    append(&ledger, &Row { blame: "bad\tcase\nhere".to_owned(), ..row("t2", "rustc 1.95.0", Outcome::Failed) }).unwrap();
    assert_eq!(rows(&ledger).len(), 2);
    assert_eq!(rows(&ledger)[1].blame, "bad case here");
    assert_eq!(proven(&ledger, "t1", "cargo test -p ra-check -q", "rustc 1.95.0"), Some(written));
    assert_eq!(proven(&ledger, "t2", "cargo test -p ra-check -q", "rustc 1.95.0"), None);
}

#[test]
fn a_different_toolchain_stamp_is_not_proven() {
    let ledger = path(&scratch("stamp"));
    append(&ledger, &row("t1", "rustc 1.95.0", Outcome::Passed)).unwrap();
    assert_eq!(proven(&ledger, "t1", "cargo test -p ra-check -q", "rustc 1.96.0"), None);
}

#[test]
fn a_changed_file_changes_the_tree_and_leaves_the_index_alone() {
    let root = scratch("tree");
    git(&root, &["init", "-q"], None).unwrap();
    fs::write(root.join("a.txt"), "one").unwrap();
    let first = tree_of(&root).unwrap();
    assert_eq!(tree_of(&root).unwrap(), first);
    fs::write(root.join("a.txt"), "two").unwrap();
    let second = tree_of(&root).unwrap();
    assert_ne!(first, second);
    // nothing was staged in the real index
    assert_eq!(git(&root, &["diff", "--cached", "--name-only"], None).unwrap(), "");
    assert_eq!(git(&root, &["ls-files"], None).unwrap(), "");
}
