//! A walk reads the project's CI on main once an hour through gh: the latest completed run of
//! each workflow is kept for `rc status` and the dashboard, and a failure or a timeout that no
//! green run has superseded files one priority-1 card naming the workflow, the job, the failing
//! step and the first failing test of `gh run view --log-failed`.

use std::sync::{Arc, Mutex};

use ra_console::agent::{Gh, ci_due, ci_failure, ci_latest, read_ci_with};
use ra_console::hub::{CI_FILE, ci_state};
use ra_roadmap::store::Store;

fn run(id: u64, workflow: &str, status: &str, conclusion: &str, sha: &str) -> String {
    format!(
        r#"{{"databaseId":{id},"workflowName":"{workflow}","headSha":"{sha}","status":"{status}","conclusion":"{conclusion}","url":"https://github.com/o/r/actions/runs/{id}","displayTitle":"the commit {sha}"}}"#
    )
}

fn list(runs: &[String]) -> String {
    format!("[{}]", runs.join(","))
}

fn failed_log() -> String {
    [
        "test\tRun cargo test\t2026-11-12T10:00:00.1000000Z running 2 tests",
        "test\tRun cargo test\t2026-11-12T10:00:01.1000000Z test hub::tests::keeps ... ok",
        "test\tRun cargo test\t2026-11-12T10:00:02.1000000Z test agent::tests::a_stale_test ... \u{1b}[31mFAILED\u{1b}[0m",
        "test\tRun cargo test\t2026-11-12T10:00:03.1000000Z thread 'agent::tests::a_stale_test' panicked at src/agent/tests.rs:9:5:",
        "test\tRun cargo test\t2026-11-12T10:00:04.1000000Z test result: FAILED. 1 passed; 1 failed",
    ]
    .join("\n")
}

#[test]
fn the_latest_completed_run_of_each_workflow_speaks_for_it() {
    let json = list(&[
        run(9, "build", "in_progress", "", "c3"),
        run(8, "build", "completed", "success", "c2"),
        run(7, "build", "completed", "failure", "c1"),
        run(6, "release", "completed", "timed_out", "c1"),
    ]);
    let latest = ci_latest(&json);
    assert_eq!(latest.len(), 2, "{latest:?}");
    // the run still going is passed over, and the green one supersedes the older failure
    assert_eq!((latest[0].workflow.as_str(), latest[0].conclusion.as_str(), latest[0].id), ("build", "success", 8));
    assert!(!latest[0].red());
    assert_eq!((latest[1].workflow.as_str(), latest[1].conclusion.as_str()), ("release", "timed_out"));
    assert!(latest[1].red(), "a timeout is red");
    assert!(ci_latest("not json").is_empty());
}

#[test]
fn the_failed_log_names_the_job_the_step_and_the_first_failing_test() {
    let failure = ci_failure(&failed_log());
    assert_eq!(failure.job.as_deref(), Some("test"));
    assert_eq!(failure.step.as_deref(), Some("Run cargo test"));
    assert_eq!(failure.test.as_deref(), Some("agent::tests::a_stale_test"));
    assert!(failure.error.contains("a_stale_test"), "{failure:?}");
}

#[tokio::test]
async fn a_red_main_files_one_card_until_a_green_run_supersedes_it() {
    let hub = tempfile::tempdir().expect("a hub dir");
    // SAFETY: no other test of this binary sets or reads the environment
    unsafe { std::env::set_var("RHIPPER_HUB", hub.path()) };
    let dir = tempfile::tempdir().expect("a tempdir");
    let root = dir.path();
    let store = Store::open(&root.join("rhipper.md"), "Probe").await.expect("the store");

    let runs = Arc::new(Mutex::new(list(&[
        run(2, "build", "completed", "failure", "abc1234def"),
        run(1, "lint", "completed", "success", "abc1234def"),
    ])));
    let calls: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let (seen, logged) = (runs.clone(), calls.clone());
    let gh: Gh = Arc::new(move |args: &[&str]| {
        logged.lock().expect("the lock").push(args.join(" "));
        match args {
            ["auth", "status"] => Some(String::new()),
            ["run", "list", ..] => {
                assert!(args.windows(2).any(|w| w == ["--branch", "main"]), "{args:?}");
                Some(seen.lock().expect("the lock").clone())
            }
            ["run", "view", _, "--log-failed"] => Some(failed_log()),
            _ => None,
        }
    });

    let hour = 3600;
    let now = 1_800_000_000;
    assert!(ci_due(root, now), "never read, the CI is due");
    let first = read_ci_with(&store, now, gh.clone()).await;
    assert!(first.iter().any(|l| l.contains("filed") && l.contains("build")), "{first:?}");
    let state = ci_state(root).expect("the read is kept");
    assert!(state.red, "{state:?}");
    assert!(state.line.contains("✗ build failure at abc1234") && state.line.contains("✓ lint"), "{}", state.line);
    assert!(std::fs::read_to_string(root.join(CI_FILE)).expect("the CI file").contains("build\tfailure\tabc1234def"));

    let cards = || async { store.all().await.expect("the roadmap").into_iter().filter(|n| n.title.starts_with("CI ")).collect::<Vec<_>>() };
    let filed = cards().await;
    assert_eq!(filed.len(), 1, "{first:?}");
    let card = &filed[0];
    assert_eq!(card.priority, Some(1));
    assert!(card.title.contains("build") && card.title.contains("agent::tests::a_stale_test"), "{}", card.title);
    let objective = card.objective.as_deref().unwrap_or_default();
    for word in ["build", "Job: test", "Step: Run cargo test", "First failing test: agent::tests::a_stale_test", "abc1234def"] {
        assert!(objective.contains(word), "{word} in {objective}");
    }

    // within the hour gh is not called again
    let before = calls.lock().expect("the lock").len();
    assert!(!ci_due(root, now + 60));
    assert!(read_ci_with(&store, now + 60, gh.clone()).await.is_empty());
    assert_eq!(calls.lock().expect("the lock").len(), before);

    // still red an hour later: the streak has its card already
    let again = read_ci_with(&store, now + hour, gh.clone()).await;
    assert!(!again.iter().any(|l| l.contains("filed")), "{again:?}");
    assert_eq!(cards().await.len(), 1);

    // a green run supersedes the failure; a later timeout is a new red and files again
    *runs.lock().expect("the lock") =
        list(&[run(3, "build", "completed", "success", "fed4321"), run(1, "lint", "completed", "success", "abc1234def")]);
    let green = read_ci_with(&store, now + 2 * hour, gh.clone()).await;
    assert!(!green.iter().any(|l| l.contains("filed")), "{green:?}");
    assert!(!ci_state(root).expect("the read").red);
    *runs.lock().expect("the lock") =
        list(&[run(4, "build", "completed", "timed_out", "0123456789"), run(1, "lint", "completed", "success", "abc1234def")]);
    let timed_out = read_ci_with(&store, now + 3 * hour, gh.clone()).await;
    assert!(timed_out.iter().any(|l| l.contains("filed") && l.contains("timed_out")), "{timed_out:?}");
    assert_eq!(cards().await.len(), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn no_gh_login_reads_nothing_and_files_nothing() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let root = dir.path();
    let store = Store::open(&root.join("rhipper.md"), "Probe").await.expect("the store");
    let gh: Gh = Arc::new(|_: &[&str]| None);
    assert!(read_ci_with(&store, 1_800_000_000, gh).await.is_empty());
    assert!(ci_state(root).is_none());
    assert!(!ci_due(root, 1_800_000_060), "the look is stamped even when gh says no");
}
