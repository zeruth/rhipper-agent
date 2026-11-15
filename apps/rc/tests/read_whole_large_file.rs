//! A whole Read of a large file is answered with its line count and its item outline, and a
//! request for a range; a ranged Read and a whole Read of a short file are not.

use std::path::Path;

use ra_console::agent::read_hint;

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a tempdir");
    std::fs::create_dir_all(dir.path().join("var/context")).expect("a context dir");
    std::fs::write(dir.path().join("var/context/721.md"), "the card's brief\n").expect("a context file");
    dir
}

fn write_items(root: &Path, name: &str, items: usize) {
    let mut text = String::new();
    for n in 0..items {
        text.push_str(&format!("fn item_{n}() {{\n    let _ = {n};\n}}\n"));
    }
    std::fs::write(root.join(name), text).expect("written");
}

fn read(path: &str, range: bool) -> serde_json::Value {
    let mut input = serde_json::json!({"file_path": path});
    if range {
        input["offset"] = serde_json::json!(1);
        input["limit"] = serde_json::json!(20);
    }
    serde_json::json!({"tool_name": "Read", "tool_input": input})
}

#[test]
fn a_whole_read_of_a_large_file_gets_its_outline_and_a_range_request() {
    let dir = project();
    write_items(dir.path(), "big.rs", 150);
    let hint = read_hint(dir.path(), &read("big.rs", false)).expect("a hint");
    assert!(hint.contains("450 lines"), "{hint}");
    assert!(hint.contains("item_0 "), "{hint}");
    assert!(hint.contains("offset"), "{hint}");
    assert!(hint.contains("limit"), "{hint}");
}

#[test]
fn a_ranged_read_passes_unchanged() {
    let dir = project();
    write_items(dir.path(), "big.rs", 150);
    assert_eq!(read_hint(dir.path(), &read("big.rs", true)), None);
}

#[test]
fn a_whole_read_of_a_short_file_passes_unchanged() {
    let dir = project();
    write_items(dir.path(), "small.rs", 10);
    assert_eq!(read_hint(dir.path(), &read("small.rs", false)), None);
}
