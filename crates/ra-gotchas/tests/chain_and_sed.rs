use ra_gotchas::Catalogue;

fn matched(command: &str) -> Option<String> {
    Catalogue::seed().matching(command).map(|g| g.id.clone())
}

#[test]
fn a_for_loop_matches_a_gotcha_that_says_one_command_per_call() {
    for command in ["for f in a.exe b.exe; do upx $f; done", "for i in 1 2 3; do echo $i; done", "for f in *.rs; do wc -l \"$f\"; done"] {
        let catalogue = Catalogue::seed();
        let gotcha = catalogue.matching(command).unwrap_or_else(|| panic!("{command} matches"));
        assert!(gotcha.message.contains("one command each") || gotcha.message.contains("one per call"), "{command}: {}", gotcha.message);
    }
}

#[test]
fn a_chain_of_several_tools_matches_a_gotcha_that_says_one_command_per_call() {
    for command in ["git status; git diff", "ls crates; cat Cargo.toml"] {
        let catalogue = Catalogue::seed();
        let gotcha = catalogue.matching(command).unwrap_or_else(|| panic!("{command} matches"));
        assert!(gotcha.message.contains("one per call"), "{command}: {}", gotcha.message);
    }
}

#[test]
fn sed_matches_a_gotcha_that_names_grep_and_read() {
    for command in ["sed -i 's/a/b/' src/lib.rs", "sed -n '1,20p' src/lib.rs", "  sed 's/a/b/' f"] {
        let catalogue = Catalogue::seed();
        let gotcha = catalogue.matching(command).unwrap_or_else(|| panic!("{command} matches"));
        assert!(gotcha.message.contains("Grep") && gotcha.message.contains("Read"), "{command}: {}", gotcha.message);
    }
}

#[test]
fn a_single_allowed_command_matches_none_of_them() {
    assert_eq!(matched("git status"), None);
}
