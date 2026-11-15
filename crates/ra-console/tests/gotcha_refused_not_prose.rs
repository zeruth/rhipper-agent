use ra_gotchas::{Catalogue, Where};

const PROSE: [&str; 3] = [
    "- **Side effect:** the hook refused nothing, but a denied build would have",
    "- **Test:** `cargo test` was refused once and ran on the second try",
    "A worktree in a project whose builds are refused gets no cargo target of its own.",
];

fn refused(place: Where, text: &str) -> bool {
    Catalogue::seed().scan(place, text).iter().any(|hit| hit.id == "refused")
}

#[test]
fn a_line_of_a_report_or_a_note_is_not_a_refused_command() {
    for line in PROSE {
        assert!(!refused(Where::Text, line), "{line}");
        assert!(!refused(Where::Answer, line), "{line}");
    }
}

#[test]
fn the_answer_of_a_denied_tool_call_is_a_refusal() {
    assert!(refused(Where::Result, "Permission to use Bash with command `rm -rf build` has been denied."));
}
