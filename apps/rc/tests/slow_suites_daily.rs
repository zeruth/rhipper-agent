//! A project's slow suites run on main once a day, at the first walk start after a day: what
//! each passed is recorded in `var/slow.log`, and a test that passed last time and fails now
//! files a priority-1 card naming the test, its first failure lines and the landings since.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ra_console::agent::{SLOW_LOG, Worker, passed_test_names, slow_due, slow_suites};
use ra_roadmap::store::Store;

const SUITE: &str = "cargo test -p ras --features oracle";

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(["-c", "user.name=test", "-c", "user.email=test@example.com"])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn green() -> String {
    "running 2 tests\ntest oracle::keeps ... ok\ntest oracle::rhtest_stl_clang_runs_as_c ... ok\n\ntest result: ok. 2 passed; 0 failed\n"
        .to_owned()
}

fn red() -> String {
    [
        "running 2 tests",
        "test oracle::keeps ... ok",
        "test oracle::rhtest_stl_clang_runs_as_c ... FAILED",
        "",
        "failures:",
        "",
        "---- oracle::rhtest_stl_clang_runs_as_c stdout ----",
        "thread 'oracle::rhtest_stl_clang_runs_as_c' panicked at tests/oracle.rs:41:5:",
        "the C run printed 3, the oracle 4",
        "",
        "failures:",
        "    oracle::rhtest_stl_clang_runs_as_c",
        "",
        "test result: FAILED. 1 passed; 1 failed",
        "",
    ]
    .join("\n")
}

#[test]
fn passed_names_are_read_from_cargo_and_gradle_lines() {
    let names = passed_test_names("test a::b ... ok\ntest a::c ... FAILED\nFooTest > bar() PASSED\n");
    assert_eq!(names, vec!["a::b".to_owned(), "FooTest.bar".to_owned()]);
}

#[tokio::test]
async fn a_test_that_passed_yesterday_and_fails_today_files_a_priority_one_card() {
    let hub = tempfile::tempdir().expect("a hub dir");
    // SAFETY: the only async test of this binary, and the other reads no environment
    unsafe { std::env::set_var("RHIPPER_HUB", hub.path()) };
    let dir = tempfile::tempdir().expect("a tempdir");
    let root = dir.path();
    std::fs::write(root.join("rhipper.toml"), format!("[worker]\nslow = [\"{SUITE}\"]\n")).expect("the table");
    std::fs::write(root.join("a.txt"), "a").expect("a file");
    git(root, &["init", "-q"]);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "the green base"]);
    let store = Store::open(&root.join("rhipper.md"), "Probe").await.expect("the store");

    let broken = Arc::new(AtomicBool::new(false));
    let seen = broken.clone();
    let script: ra_tools::Script = Arc::new(move |command: &str| {
        assert_eq!(command, SUITE);
        Ok(if seen.load(Ordering::SeqCst) { (101, red()) } else { (0, green()) })
    });
    let worker = Worker::new("z-ai/glm-5.3-flash").with_script(script);

    let day = 86_400;
    let now = 1_800_000_000;
    assert!(slow_due(root, now), "never run, the suites are due");
    let first = slow_suites(&store, &worker, now).await;
    assert!(first.iter().any(|l| l.contains("2 passed, 0 failed")), "{first:?}");
    let log = std::fs::read_to_string(root.join(SLOW_LOG)).expect("the log");
    assert!(log.contains("oracle::rhtest_stl_clang_runs_as_c"), "{log}");
    assert!(!slow_due(root, now + 3600), "an hour later they are not due again");
    assert!(slow_suites(&store, &worker, now + 3600).await.is_empty());

    std::fs::write(root.join("b.txt"), "b").expect("a file");
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "the landing that broke it"]);
    broken.store(true, Ordering::SeqCst);
    assert!(slow_due(root, now + day + 1));
    let second = slow_suites(&store, &worker, now + day + 1).await;
    assert!(second.iter().any(|l| l.contains("filed") && l.contains("rhtest_stl_clang_runs_as_c")), "{second:?}");

    let cards: Vec<_> =
        store.all().await.expect("the roadmap").into_iter().filter(|n| n.title.contains("rhtest_stl_clang_runs_as_c")).collect();
    assert_eq!(cards.len(), 1, "one card for the one broken test: {second:?}");
    let card = &cards[0];
    assert_eq!(card.priority, Some(1));
    let objective = card.objective.as_deref().unwrap_or_default();
    assert!(objective.contains("the C run printed 3, the oracle 4"), "{objective}");
    assert!(objective.contains("the landing that broke it"), "{objective}");
    assert!(!objective.contains("the green base"), "only the landings since the green run: {objective}");
    assert!(card.acceptance.as_deref().unwrap_or_default().contains(&format!("{SUITE} oracle::rhtest_stl_clang_runs_as_c")));

    // still failing the day after, it passed in no run since: no second card
    let third = slow_suites(&store, &worker, now + 2 * day + 2).await;
    assert!(!third.iter().any(|l| l.contains("filed")), "{third:?}");
}
