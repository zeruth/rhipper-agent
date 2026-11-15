use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use ra_roadmap::model::NodeKind;

use super::*;
use crate::{pin, project};

/// The files every card of the bundle's roadmap names.
fn card_files(bundle: &Bundle) -> BTreeSet<String> {
    let nodes = pin::roadmap(bundle).expect("the roadmap parses");
    nodes.into_iter().filter(|n| n.kind == NodeKind::Task).flat_map(|n| n.files).collect()
}

/// The embedded project written to `dir`, with the solved tree over it when `solved`.
fn project_at(dir: &Path, solved: bool) -> Bundle {
    let bundle = Bundle::embedded();
    project::write(&bundle, dir).expect("the project is written");
    if solved {
        lay_solved(&bundle, dir).expect("the solved tree is written");
    }
    bundle
}

/// The checks of `verdicts` that did not pass.
fn failing(verdicts: &[TreeVerdict]) -> Vec<&'static str> {
    verdicts.iter().filter(|v| v.verdict != Verdict::Pass).map(|v| v.check).collect()
}

#[test]
fn a_test_name_is_the_function_after_each_test_attribute() {
    let source = "fn helper() {}\n\n#[test]\nfn first() {\n    fn inner() {}\n}\n\n#[test]\n#[ignore]\nfn second_one() {}\n";
    assert_eq!(test_names(source), ["first", "second_one"]);
}

#[test]
fn cargo_output_gives_one_verdict_a_test_and_a_build_failure_gives_compile_to_each() {
    let names = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
    let stdout = "\nrunning 2 tests\ntest b ... FAILED\ntest a ... ok\n\ntest result: FAILED. 1 passed; 1 failed\n";
    let verdicts: Vec<Verdict> = read_verdicts(&names, stdout).into_iter().map(|v| v.verdict).collect();
    assert_eq!(verdicts, [Verdict::Pass, Verdict::Fail, Verdict::Fail]);
    let verdicts: Vec<Verdict> = read_verdicts(&names, "").into_iter().map(|v| v.verdict).collect();
    assert_eq!(verdicts, [Verdict::Compile; 3]);
}

#[test]
fn every_hidden_check_and_every_solved_file_is_in_the_bundle() {
    let bundle = Bundle::embedded();
    let manifest = pin::manifest(&bundle).expect("the manifest reads");
    for card in &manifest.cards {
        let source = bundle.get(&hidden_path(&card.key)).expect("the hidden check is embedded");
        assert!(!test_names(std::str::from_utf8(source).expect("UTF-8")).is_empty(), "{} has tests", card.key);
    }
    let solved: Vec<&str> = bundle.paths().filter_map(|path| path.strip_prefix(SOLVED)).collect();
    assert_eq!(solved.len(), 9);
    let files = card_files(&bundle);
    assert!(solved.iter().all(|path| files.contains(*path)), "every solved file is a card's: {solved:?}");
}

#[test]
#[ignore = "runs cargo on a copy of the project"]
fn hidden_checks_each_find_the_base_undone() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let bundle = project_at(dir.path(), false);
    for card in pin::manifest(&bundle).expect("the manifest reads").cards {
        let verdicts = hidden(&bundle, &card, dir.path()).expect("the hidden check runs");
        assert!(!verdicts.is_empty(), "{} has verdicts", card.key);
        assert!(verdicts.iter().any(|v| v.verdict != Verdict::Pass), "{} passes on the base: {verdicts:?}", card.key);
    }
}

#[test]
#[ignore = "runs cargo on a copy of the project"]
fn hidden_checks_and_tree_checks_all_pass_with_the_solved_tree_over_the_base() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let bundle = project_at(dir.path(), true);
    let verdicts = tree(dir.path(), &bundle, &card_files(&bundle)).expect("the tree checks run");
    assert_eq!(verdicts.iter().map(|v| v.check).collect::<Vec<_>>(), TREE_CHECKS);
    assert_eq!(failing(&verdicts), Vec::<&str>::new(), "{verdicts:?}");
    for card in pin::manifest(&bundle).expect("the manifest reads").cards {
        let verdicts = hidden(&bundle, &card, dir.path()).expect("the hidden check runs");
        assert!(!verdicts.is_empty(), "{} has verdicts", card.key);
        assert!(verdicts.iter().all(|v| v.verdict == Verdict::Pass), "{}: {verdicts:?}", card.key);
    }
}

#[test]
#[ignore = "runs cargo on a copy of the project"]
fn hidden_checks_of_the_tree_turn_one_each_on_a_marker_an_allow_and_a_stray_file() {
    let cases = [
        ("whole", "crates/tally/src/lib.rs", "\nconst _: &str = \"\n=======\n\";\n"),
        ("quiet", "crates/tally/src/lib.rs", "\n#[allow(dead_code)]\nfn unused() {}\n"),
        ("scope", "notes.txt", "a note\n"),
    ];
    for (check, path, added) in cases {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let bundle = project_at(dir.path(), true);
        let file = dir.path().join(path);
        let mut text = fs::read_to_string(&file).unwrap_or_default();
        text.push_str(added);
        fs::write(&file, text).expect("the change is written");

        let verdicts = tree(dir.path(), &bundle, &card_files(&bundle)).expect("the tree checks run");
        assert_eq!(failing(&verdicts), [check], "{verdicts:?}");
        let fault = verdicts.iter().find(|v| v.check == check).expect("the check ran");
        assert_eq!(fault.paths, [path], "{check}");
    }
}
