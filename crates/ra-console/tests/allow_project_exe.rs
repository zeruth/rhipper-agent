use ra_console::router::Router;

fn router() -> Router {
    Router::parse("[worker]\nbinaries = [\"testdata/rhtest/*.exe\"]\n").expect("the table parses")
}

#[test]
fn a_listed_test_binary_runs_without_approval() {
    let worker = router().worker;
    assert!(worker.allows("testdata/rhtest/core.exe --quiet"));
    assert!(worker.allows("./testdata/rhtest/core.exe"));
    assert!(worker.allows(r".\testdata\rhtest\core.exe"));
}

#[test]
fn a_binary_outside_the_worktree_or_matching_no_pattern_is_refused() {
    let worker = router().worker;
    assert!(!worker.allows("testdata/other/core.exe"));
    assert!(!worker.allows("testdata/rhtest/sub/core.exe"));
    assert!(!worker.allows("testdata/rhtest/core.dll"));
    assert!(!worker.allows("../testdata/rhtest/core.exe"));
    assert!(!worker.allows("testdata/../../x/rhtest/core.exe"));
    assert!(!worker.allows("/testdata/rhtest/core.exe"));
    assert!(!worker.allows("C:/work/testdata/rhtest/core.exe"));
}

#[test]
fn a_project_without_the_setting_allows_no_binary() {
    let worker = Router::parse("[worker]\n").expect("parses").worker;
    assert!(!worker.allows("testdata/rhtest/core.exe"));
}
