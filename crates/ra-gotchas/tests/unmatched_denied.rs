//! The three refused commands the journal logged as unmatched each match a gotcha now.

use ra_gotchas::Catalogue;

fn matched(command: &str) -> String {
    Catalogue::seed().matching(command).unwrap_or_else(|| panic!("no gotcha matches `{command}`")).id.clone()
}

#[test]
fn unmatched_denied_semicolon_chain_names_one_command_per_call() {
    let gotcha = Catalogue::seed().matching("git status --short; git log --oneline -3; git diff --stat HEAD").cloned().expect("a gotcha");
    assert_eq!(gotcha.id, "piped-or-chained");
    assert!(gotcha.message.contains("one per call"));
}

#[test]
fn unmatched_denied_path_outside_the_workspace_names_the_workspace() {
    for command in [
        "tail -n 40 ../../pair.log",
        "cat D:/rhipper/target/ra/out.log",
        "stat D:\\rhipper\\notes.log",
        "Get-Content D:\\rhipper\\notes.log",
    ] {
        assert_eq!(matched(command), "path-outside-workspace", "{command}");
    }
    let gotcha = Catalogue::seed().get("path-outside-workspace").cloned().expect("a gotcha");
    assert!(gotcha.message.contains("workspace") && gotcha.message.contains("Read"));
}

#[test]
fn unmatched_denied_pipe_after_stderr_redirect_names_one_command_per_call() {
    assert_eq!(matched("rc hold -- cargo test -p ra-console -q 2>&1 | tail -30"), "piped-or-chained");
}
