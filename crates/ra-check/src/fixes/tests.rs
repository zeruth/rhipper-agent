use serde_json::json;

use super::*;

const SOURCE: &str = "pub fn empty(a: &[u8], b: &[u8]) -> bool {\n    a.len() == 0 && b.len() == 0\n}\n\npub fn more(c: &[u8]) -> bool {\n    c.len() == 0\n}\n";

/// A cargo `compiler-message` line suggesting `replacement` for the first `needle` of SOURCE.
fn suggestion(file: &str, needle: &str, replacement: &str, applicability: &str) -> String {
    let start = SOURCE.find(needle).expect("the needle is in the fixture");
    let end = start + needle.len();
    let line = SOURCE[..start].matches('\n').count() + 1;
    let span = |replacement: Option<&str>, applicability: Option<&str>| {
        json!({
            "file_name": file, "byte_start": start, "byte_end": end,
            "line_start": line, "line_end": line, "column_start": 1, "column_end": 1,
            "is_primary": true, "suggested_replacement": replacement, "suggestion_applicability": applicability,
        })
    };
    json!({
        "reason": "compiler-message",
        "package_id": "fixture 0.1.0",
        "message": {
            "message": "length comparison to zero", "level": "warning",
            "spans": [span(None, None)],
            "children": [{
                "message": "using `is_empty` is clearer", "level": "help",
                "spans": [span(Some(replacement), Some(applicability))], "children": [],
            }],
        },
    })
    .to_string()
}

fn changed() -> Vec<(String, u32, u32)> {
    vec![("src/lib.rs".to_owned(), 1, 3)]
}

#[test]
fn fixes_on_one_line_are_both_applied_and_one_outside_the_ranges_is_left() {
    let json = [
        suggestion("src/lib.rs", "a.len() == 0", "a.is_empty()", "MachineApplicable"),
        suggestion("src/lib.rs", "b.len() == 0", "b.is_empty()", "MachineApplicable"),
        suggestion("src/lib.rs", "c.len() == 0", "c.is_empty()", "MachineApplicable"),
        r#"{"reason":"build-finished","success":true}"#.to_owned(),
    ]
    .join("\n");
    let found = fixes(&json, &changed());
    assert_eq!(found.len(), 2);
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::create_dir(dir.path().join("src")).expect("src");
    std::fs::write(dir.path().join("src/lib.rs"), SOURCE).expect("the fixture");
    assert_eq!(apply(dir.path(), &found).expect("applied"), 2);
    let fixed = std::fs::read_to_string(dir.path().join("src/lib.rs")).expect("read back");
    assert!(fixed.contains("    a.is_empty() && b.is_empty()\n"), "{fixed}");
    assert!(fixed.contains("    c.len() == 0\n"), "{fixed}");
}

#[test]
fn fixes_not_machine_applicable_are_not_kept() {
    let json = suggestion("src/lib.rs", "a.len() == 0", "a.is_empty()", "MaybeIncorrect");
    assert!(fixes(&json, &changed()).is_empty());
}

#[test]
fn fixes_named_with_backslashes_match_the_diff_path_and_a_repeat_is_kept_once() {
    let line = suggestion("crates\\x\\src\\lib.rs", "a.len() == 0", "a.is_empty()", "MachineApplicable");
    let json = format!("{line}\n{line}");
    let found = fixes(&json, &[("crates/x/src/lib.rs".to_owned(), 2, 2)]);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].file, "crates/x/src/lib.rs");
    assert_eq!(found[0].lines, (2, 2));
}
