//! A whole-file Read of a long file is refused with a gotcha naming `offset` and `limit`, any
//! long file, in a worktree too, and through a shell's `cat` as well.

use ra_gotchas::{Catalogue, LONG_FILE, ReadCall, Verdict};

fn call(lines: usize, ranged: bool, items: Option<&[String]>) -> ReadCall<'_> {
    ReadCall { file: "apps/rc/src/main.rs", ranged, lines, items }
}

#[test]
fn a_whole_read_of_a_large_file_names_its_line_count_and_the_ranged_form() {
    let items: Vec<String> = Vec::new();
    let Verdict::Deny { id, reason } = Catalogue::seed().read(call(2253, false, Some(&items))) else {
        panic!("a whole read of a large file is refused");
    };
    assert_eq!(id, "whole-file-read");
    assert!(reason.contains("2253 lines"), "{reason}");
    assert!(reason.contains("offset"), "{reason}");
    assert!(reason.contains("limit"), "{reason}");
}

#[test]
fn a_whole_read_of_a_file_the_card_does_not_name_is_refused_too() {
    let Verdict::Deny { id, reason } = Catalogue::seed().read(call(2253, false, None)) else {
        panic!("a long file outside the card's outline is refused all the same");
    };
    assert_eq!(id, "whole-file-read");
    assert!(reason.contains("2253 lines"), "{reason}");
}

/// A worktree as a walk's run has it: no `var/context`, no catalogue, a long file the card does
/// not name; a whole Read and a `cat` of it are refused, and the reason counts as the gotcha.
#[test]
fn the_hook_refuses_a_whole_read_in_a_worktree_with_no_context_file() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let worktree = dir.path().join("var").join("worktrees").join("86");
    std::fs::create_dir_all(worktree.join("src")).expect("a source dir");
    let long: String = (0..900).map(|n| format!("fn item_{n}() {{}}\n")).collect();
    std::fs::write(worktree.join("src/psp.rs"), long).expect("a long file");
    for (tool, input) in [
        ("Read", serde_json::json!({"file_path": worktree.join("src/psp.rs").display().to_string()})),
        ("Bash", serde_json::json!({"command": "cat src/psp.rs"})),
        ("PowerShell", serde_json::json!({"command": "Get-Content -Path src/psp.rs"})),
    ] {
        let call = serde_json::json!({"tool_name": tool, "tool_input": input, "cwd": worktree});
        let reason = ra_console::agent::read_refusal(&worktree, &call).unwrap_or_else(|| panic!("{tool} is refused"));
        assert!(reason.contains("src/psp.rs is 900 lines"), "{reason}");
        assert!(reason.contains("its items: item_0 "), "{reason}");
        let hits = Catalogue::seed().refused_read(&reason);
        assert_eq!(hits.iter().map(|h| h.id.as_str()).collect::<Vec<_>>(), ["whole-file-read"]);
    }
    let ranged = serde_json::json!({"tool_name": "Read", "tool_input": {"file_path": "src/psp.rs", "offset": 1, "limit": 40}});
    assert_eq!(ra_console::agent::read_refusal(&worktree, &ranged), None);
}

#[test]
fn a_ranged_read_and_a_small_file_pass() {
    let items: Vec<String> = Vec::new();
    let catalogue = Catalogue::seed();
    assert_eq!(catalogue.read(call(2253, true, Some(&items))), Verdict::Pass);
    assert_eq!(catalogue.read(call(LONG_FILE, false, Some(&items))), Verdict::Pass);
}
