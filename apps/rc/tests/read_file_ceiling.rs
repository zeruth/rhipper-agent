//! `read_file` on a file over the size ceiling answers with its outline and asks for a range.

use ra_providers::ToolCall;
use ra_tools::Toolbox;

fn read(toolbox: &Toolbox, arguments: &str) -> String {
    let runtime = tokio::runtime::Builder::new_current_thread().build().expect("a runtime");
    let call = ToolCall { id: "1".to_owned(), name: "read_file".to_owned(), arguments: arguments.to_owned() };
    runtime.block_on(toolbox.execute(&call))
}

#[test]
fn a_whole_file_read_over_the_ceiling_returns_the_outline_and_a_range_note() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let mut text = String::new();
    for n in 0..400 {
        text.push_str(&format!("## Section {n}\nsome body text of the section that fills the file up, line {n}\n"));
    }
    assert!(text.chars().count() > 20_000);
    std::fs::write(dir.path().join("big.md"), &text).expect("written");
    let toolbox = Toolbox::new(dir.path()).expect("a toolbox");

    let whole = read(&toolbox, r#"{"path": "big.md"}"#);
    assert!(whole.contains("outline"), "{whole}");
    assert!(whole.contains("    1│## Section 0"), "{whole}");
    assert!(whole.contains("offset"), "{whole}");
    assert!(!whole.contains("some body text"), "{whole}");

    let ranged = read(&toolbox, r#"{"path": "big.md", "offset": 1, "limit": 4}"#);
    assert!(ranged.contains("some body text"), "{ranged}");
    assert!(!ranged.contains("outline"), "{ranged}");
}

#[test]
fn a_small_file_still_reads_whole() {
    let dir = tempfile::tempdir().expect("a tempdir");
    std::fs::write(dir.path().join("small.md"), "# Title\nbody\n").expect("written");
    let toolbox = Toolbox::new(dir.path()).expect("a toolbox");
    let whole = read(&toolbox, r#"{"path": "small.md"}"#);
    assert!(whole.contains("body"), "{whole}");
}
