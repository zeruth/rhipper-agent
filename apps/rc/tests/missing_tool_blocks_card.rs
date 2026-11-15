//! A report whose `[machine]` note says a tool is missing holds the card for the person.

use ra_console::agent::missing_tool_line;

#[test]
fn a_machine_note_naming_a_missing_tool_is_the_reason() {
    let report = "Done what I could.\n\n## Faster next time\n- [machine] csmith is not installed in WSL Ubuntu-24.04\n";
    assert_eq!(missing_tool_line(report).as_deref(), Some("[machine] csmith is not installed in WSL Ubuntu-24.04"));
}

#[test]
fn a_report_without_such_a_note_holds_nothing() {
    assert_eq!(missing_tool_line("- [machine] a first compile runs past 120 s\n- csmith is not installed\n"), None);
    assert_eq!(missing_tool_line("all done\n"), None);
}

#[test]
fn the_reason_is_clipped() {
    let long = format!("- [machine] the binary is missing {}", "x".repeat(300));
    assert_eq!(missing_tool_line(&long).map(|line| line.chars().count()), Some(200));
}
