//! The handoff marks a card file over 12,000 characters and says to read it by range; a smaller
//! file gets no such line.

use ra_context::{TaskContext, large_files_note};

#[test]
fn only_a_card_file_over_the_limit_is_marked_for_range_reads() {
    let dir = tempfile::tempdir().expect("a tempdir");
    std::fs::write(dir.path().join("big.rs"), "x".repeat(20_000)).expect("written");
    std::fs::write(dir.path().join("small.rs"), "x".repeat(2_000)).expect("written");
    let files = vec!["big.rs".to_owned(), "small.rs".to_owned()];

    let mut task = TaskContext::open(dir.path(), 761);
    task.brief = "the brief".to_owned();
    task.outlines = large_files_note(dir.path(), &files);
    let prompt = task.prompt();

    assert!(prompt.contains("big.rs (20000 characters)"), "{prompt}");
    assert!(prompt.contains("outline first"), "{prompt}");
    assert!(prompt.contains("line ranges"), "{prompt}");
    assert!(!prompt.contains("small.rs"), "{prompt}");
}
