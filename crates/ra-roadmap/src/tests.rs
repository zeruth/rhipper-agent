use crate::check::{WORKSPACE_TESTS, commands, crate_of};

fn files(list: &[&str]) -> Vec<String> {
    list.iter().map(|&f| f.to_owned()).collect()
}

#[test]
fn the_files_of_a_card_name_one_test_run_per_crate() {
    let runs = commands(&files(&["crates/ra-tools/src/commands.rs", "apps/rc/src/main.rs"]));
    assert_eq!(runs, ["cargo test -p ra-tools -q", "cargo test -p rc -q"]);
}

#[test]
fn an_empty_file_list_runs_the_workspace_tests() {
    assert_eq!(commands(&[]), [WORKSPACE_TESTS]);
    assert_eq!(commands(&files(&["rhipper.md", "docs/plan.md"])), [WORKSPACE_TESTS]);
}

#[test]
fn a_crate_touched_twice_runs_once_and_a_backslash_path_reads_the_same() {
    let runs = commands(&files(&["crates/ra-roadmap/src/check.rs", "crates\\ra-roadmap\\src\\tests.rs", "crates/ra-console/tests/run.rs"]));
    assert_eq!(runs, ["cargo test -p ra-roadmap -q", "cargo test -p ra-console -q"]);
}

#[test]
fn a_root_level_source_file_names_no_crate() {
    assert_eq!(crate_of("src/main.rs"), None);
    assert_eq!(crate_of("main.rs"), None);
    assert_eq!(crate_of("apps/rc/src/main.rs"), Some("rc"));
}
