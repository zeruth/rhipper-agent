//! A build's result names the files edited after it started.

#[path = "../src/stale_edits.rs"]
mod stale_edits;

use std::time::{Duration, SystemTime};

#[test]
fn a_result_names_the_files_edited_after_the_build_started() {
    let dir = tempfile::tempdir().expect("a tempdir");
    std::fs::create_dir_all(dir.path().join("src")).expect("a src dir");
    std::fs::create_dir_all(dir.path().join("var")).expect("a var dir");
    std::fs::write(dir.path().join("src").join("a.rs"), "a").expect("a file");
    std::fs::write(dir.path().join("var").join("state"), "s").expect("state");
    let started = SystemTime::now() - Duration::from_secs(60);
    let files = stale_edits::edited_since(dir.path(), started);
    assert_eq!(files, vec!["src/a.rs".to_string()]);
    let note = stale_edits::note(&files).expect("a note");
    assert!(note.contains("src/a.rs") && note.contains("not covered"), "{note}");

    let later = SystemTime::now() + Duration::from_secs(60);
    assert!(stale_edits::edited_since(dir.path(), later).is_empty());
    assert!(stale_edits::note(&[]).is_none());
}
