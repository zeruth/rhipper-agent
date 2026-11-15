//! A landing whose merged check fails only in a test the project's `[worker] regenerate` names
//! rewrites that test's file in the card's worktree, commits it on the card's branch and lands;
//! any other failure sends the card back as before.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ra_console::agent::{Worker, land};
use ra_console::router::{Regenerate, Router};
use ra_roadmap::history::Source;
use ra_roadmap::model::{NodeKind, TaskState};
use ra_roadmap::store::Store;

const TOML: &str = "[worker]\nregenerate = [{ test = \"tests/abi_snapshot.rs\", command = \"RA_SNAPSHOT=write snap --write testdata\" }]\n";

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(["-c", "user.name=test", "-c", "user.email=test@example.com"])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A project with a snapshot test and its snapshot, and card 2 implemented in its worktree on
/// `card/2`; the project's root and the worktree.
async fn project(dir: &Path) -> (Store, PathBuf) {
    std::fs::create_dir_all(dir.join("tests")).unwrap();
    std::fs::create_dir_all(dir.join("testdata")).unwrap();
    std::fs::write(dir.join("rhipper.toml"), TOML).unwrap();
    std::fs::write(dir.join(".gitignore"), "var/\n").unwrap();
    std::fs::write(dir.join("tests").join("abi_snapshot.rs"), "#[test]\nfn eh_clang_abi() {}\n").unwrap();
    std::fs::write(dir.join("testdata").join("abi.snap"), "old\n").unwrap();
    git(dir, &["init", "-q"]);
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "probe"]);
    git(dir, &["worktree", "add", "-q", "-b", "card/2", "var/worktrees/2"]);
    let tree = dir.join("var").join("worktrees").join("2");
    std::fs::write(tree.join("abi.txt"), "the card's change to the ABI\n").unwrap();
    git(&tree, &["add", "-A"]);
    git(&tree, &["commit", "-q", "-m", "the card's work"]);
    let store = Store::open(&dir.join("rhipper.md"), "Rhipper").await.expect("the store");
    let id = store
        .insert(Some(1), NodeKind::Task, "Change the ABI", None, Some("`cargo test --test abi_snapshot` passes"), 3)
        .await
        .expect("a card");
    assert_eq!(id, 2);
    store.set_state_by(id, TaskState::Implemented, Source::Run).await.expect("the state");
    (store, tree)
}

/// The check: the snapshot test passes once the merged tree holds the new snapshot, and fails
/// by name before; `other` fails another test whatever the snapshot holds. The regenerate
/// command writes the new snapshot in the worktree and says it ran.
fn script(root: PathBuf, tree: PathBuf, other: bool, ran: Arc<AtomicBool>) -> ra_tools::Script {
    Arc::new(move |command: &str| {
        if command.starts_with("snap --write") {
            ran.store(true, Ordering::SeqCst);
            std::fs::write(tree.join("testdata").join("abi.snap"), "new\n")?;
            return Ok((0, "wrote testdata/abi.snap\n".to_owned()));
        }
        // a checkout may end the line with `\r\n`
        let snapshot = std::fs::read_to_string(root.join("testdata").join("abi.snap")).unwrap_or_default();
        if other {
            Ok((101, "running 1 test\ntest unwind_order ... FAILED\n\ntest result: FAILED. 0 passed; 1 failed\n".to_owned()))
        } else if snapshot.trim_end() == "new" {
            Ok((0, "running 1 test\ntest eh_clang_abi ... ok\n\ntest result: ok. 1 passed; 0 failed\n".to_owned()))
        } else {
            Ok((101, "running 1 test\ntest eh_clang_abi ... FAILED\n\ntest result: FAILED. 0 passed; 1 failed\n".to_owned()))
        }
    })
}

#[tokio::test]
async fn a_landing_rewrites_the_snapshot_its_check_alone_failed_on_and_sends_any_other_failure_back() {
    let hub = tempfile::tempdir().expect("a hub dir");
    // SAFETY: the only test of this binary reading the environment, and nothing else runs yet
    unsafe { std::env::set_var("RHIPPER_HUB", hub.path()) };

    // the snapshot test alone fails: the landing rewrites the snapshot and lands
    let dir = tempfile::tempdir().expect("a tempdir");
    let (store, tree) = project(dir.path()).await;
    let ran = Arc::new(AtomicBool::new(false));
    let runner = Worker::new("z-ai/glm-5.3-flash").with_script(script(dir.path().to_path_buf(), tree.clone(), false, ran.clone()));
    let landed = land(&store, 2, &runner).await;
    assert!(ran.load(Ordering::SeqCst), "the snapshot was not rewritten: {landed:?}");
    let lines = landed.expect("the card lands");
    assert!(lines.iter().any(|l| l.starts_with("regenerated testdata/abi.snap on card/2: RA_SNAPSHOT=write snap")), "{lines:?}");
    assert!(lines.iter().any(|l| l.starts_with("check passed on the merged tree")), "{lines:?}");
    assert_eq!(store.read(2).await.unwrap().state, TaskState::Accepted);
    assert_eq!(git(dir.path(), &["show", "HEAD:testdata/abi.snap"]), "new");
    assert_eq!(git(dir.path(), &["show", "HEAD:abi.txt"]), "the card's change to the ABI");

    // another test fails: nothing is rewritten and the card goes back as before
    let dir = tempfile::tempdir().expect("a tempdir");
    let (store, tree) = project(dir.path()).await;
    let ran = Arc::new(AtomicBool::new(false));
    let runner = Worker::new("z-ai/glm-5.3-flash").with_script(script(dir.path().to_path_buf(), tree.clone(), true, ran.clone()));
    let error = land(&store, 2, &runner).await.expect_err("the card does not land");
    assert!(format!("{error:#}").contains("the merged tree fails the check"), "{error:#}");
    assert!(!ran.load(Ordering::SeqCst));
    assert_eq!(std::fs::read_to_string(tree.join("testdata").join("abi.snap")).unwrap().trim_end(), "old");
    assert_eq!(store.read(2).await.unwrap().state, TaskState::Implemented);
}

#[test]
fn an_entry_names_a_test_by_its_name_or_by_the_file_that_defines_it() {
    let dir = tempfile::tempdir().expect("a tempdir");
    std::fs::create_dir_all(dir.path().join("tests")).unwrap();
    std::fs::write(dir.path().join("tests").join("abi_snapshot.rs"), "#[test]\nfn eh_clang_abi() {}\n").unwrap();
    let file = Regenerate { test: "tests/abi_snapshot.rs".to_owned(), command: "snap".to_owned() };
    assert!(file.names(dir.path(), "eh_clang_abi"));
    assert!(file.names(dir.path(), "abi::eh_clang_abi"));
    assert!(!file.names(dir.path(), "clang_abi"), "a name ending in it is another test");
    assert!(!file.names(dir.path(), "unwind_order"));

    let name = Regenerate { test: "scoreboard_matches".to_owned(), command: "ras score --write testdata/bin".to_owned() };
    assert!(name.names(dir.path(), "scoreboard_matches"));
    assert!(name.names(dir.path(), "score::scoreboard_matches"));
    assert!(!name.names(dir.path(), "scoreboard_matches_not"));
    let class = Regenerate { test: "ScoreTest".to_owned(), command: "gradle score".to_owned() };
    assert!(class.names(dir.path(), "ScoreTest.ranks"));
}

#[test]
fn every_failing_test_must_be_named_and_a_failure_without_names_regenerates_nothing() {
    let worker = Router::parse(TOML).expect("the table parses").worker;
    let tree = tempfile::tempdir().expect("a tempdir");
    std::fs::create_dir_all(tree.path().join("tests")).unwrap();
    std::fs::write(tree.path().join("tests").join("abi_snapshot.rs"), "fn eh_clang_abi() {}\n").unwrap();
    let named = vec!["eh_clang_abi".to_owned()];
    assert_eq!(worker.regenerations(tree.path(), std::slice::from_ref(&named)).map(|e| e.len()), Some(1));
    assert!(worker.regenerations(tree.path(), &[vec!["eh_clang_abi".to_owned(), "unwind_order".to_owned()]]).is_none());
    assert!(worker.regenerations(tree.path(), &[named, Vec::new()]).is_none(), "a compile error names no test");
    assert!(Router::parse("[worker]\n").unwrap().worker.regenerations(tree.path(), &[vec!["eh_clang_abi".to_owned()]]).is_none());

    let (env, words) = worker.regenerate[0].words().expect("the command splits");
    assert_eq!(env, [("RA_SNAPSHOT".to_owned(), "write".to_owned())]);
    assert_eq!(words, ["snap", "--write", "testdata"]);
}
