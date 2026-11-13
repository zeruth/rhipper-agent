use ra_gotchas::Catalogue;

fn matched(command: &str) -> Option<String> {
    Catalogue::seed().matching(command).map(|g| g.id.clone())
}

#[test]
fn a_command_with_the_exit_status_variable_matches_and_says_to_drop_it() {
    let catalogue = Catalogue::seed();
    let gotcha = catalogue.matching("cargo test -q; echo $?").expect("matches");
    assert_eq!(gotcha.id, "shell-variable-command");
    assert!(gotcha.message.contains("drop `$?`"));
}

#[test]
fn a_for_loop_over_a_variable_matches_and_says_to_write_the_literal_path() {
    let catalogue = Catalogue::seed();
    let gotcha = catalogue.matching("for f in $files; do upx $f; done").expect("matches");
    assert_eq!(gotcha.id, "shell-variable-command");
    assert!(gotcha.message.contains("literal path once per file"));
}

#[test]
fn a_powershell_call_of_a_variable_matches() {
    assert_eq!(matched("$upx = 'a.exe'; & $upx --best b.exe").as_deref(), Some("shell-variable-command"));
    assert_eq!(matched("& $upx b.exe").as_deref(), Some("shell-variable-command"));
}

#[test]
fn a_literal_call_and_a_literal_loop_do_not_match() {
    assert_ne!(matched("& \"C:\\tools\\upx.exe\" b.exe").as_deref(), Some("shell-variable-command"));
    assert_ne!(matched("git status").as_deref(), Some("shell-variable-command"));
}
