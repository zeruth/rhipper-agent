//! The tests of the commands: parsing, the lifecycle names and the tree walk.

use super::*;
use ra_roadmap::store::Store;

/// The worker of the tests, on the badge model; no test talks to a network.
fn worker() -> Worker {
    Worker::new("z-ai/glm-5.3-flash")
}

async fn store() -> Store {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    // the tempdir lives until the store is dropped from the test's future; leak it for the test
    std::mem::forget(dir);
    store
}

#[tokio::test]
async fn adds_a_project_and_shows_it() {
    let store = store().await;
    let (out, _) = run("add task Rhipper parent 1", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["#2 Rhipper"]);
    let (out, _) = run("show", &store, &worker()).await.unwrap();
    assert!(out.iter().any(|l| l.contains("#2 Rhipper")));
}

#[tokio::test]
async fn adds_a_task_under_its_parent() {
    let store = store().await;
    let (out, _) = run("add task Write the decoder parent 1", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["#2 Write the decoder"]);
}

#[tokio::test]
async fn a_task_needs_a_parent() {
    let store = store().await;
    let error = run("add task Orphan", &store, &worker()).await.unwrap_err();
    assert_eq!(format!("{error:#}"), "a node below a project needs parent <id>");
}

#[tokio::test]
async fn a_project_takes_no_parent() {
    let store = store().await;
    let error = run("add project Root parent 7", &store, &worker()).await.unwrap_err();
    assert_eq!(format!("{error:#}"), "a project has no parent");
}

#[tokio::test]
async fn sets_states() {
    let store = store().await;
    run("add task T parent 1", &store, &worker()).await.unwrap();
    let (out, _) = run("set 2 in_progress", &store, &worker()).await.unwrap();
    assert_eq!(out, vec!["#2 is now in_progress"]);
    let error = run("set 2 nonsense", &store, &worker()).await.unwrap_err();
    assert!(format!("{error:#}").starts_with("the state is defined, ready, "), "{error:#}");
}

#[tokio::test]
async fn completes_names() {
    assert_eq!(complete("se"), ["set", "seed"]);
    assert_eq!(complete("q"), ["quit"]);
}

#[tokio::test]
async fn show_indents_by_depth_and_names_a_missing_node() {
    let store = store().await;
    run("add task T parent 1", &store, &worker()).await.unwrap();
    let (out, _) = run("show", &store, &worker()).await.unwrap();
    assert!(out[0].starts_with("#1 "), "{out:?}");
    assert!(out[1].starts_with("  #2 T"), "{out:?}");
    let error = run("show 9", &store, &worker()).await.unwrap_err();
    assert_eq!(format!("{error:#}"), "no node 9");
}

#[tokio::test]
async fn add_names_the_kinds_on_a_bad_one() {
    let store = store().await;
    let error = run("add epic T parent 1", &store, &worker()).await.unwrap_err();
    assert_eq!(format!("{error:#}"), "the kind is project, milestone, section, task or subtask");
}

#[tokio::test]
async fn seed_fills_an_empty_roadmap_and_refuses_a_second_time() {
    let store = store().await;
    let (out, _) = run("seed", &store, &worker()).await.unwrap();
    assert!(out[0].contains("planted"), "{}", out[0]);
    let error = run("seed", &store, &worker()).await.unwrap_err();
    assert!(format!("{error:#}").contains("already holds nodes"), "{error:#}");
    let (out, _) = run("show", &store, &worker()).await.unwrap();
    assert!(out.iter().any(|l| l.contains("ping")), "the seed holds the ping task: {out:?}");
}

#[tokio::test]
async fn run_refuses_a_project_and_names_a_missing_key() {
    let store = store().await;
    let error = run("run 1", &store, &worker()).await.unwrap_err();
    assert!(format!("{error:#}").contains("is a project"), "{error:#}");
    run("add task T parent 1", &store, &worker()).await.unwrap();
    // this machine may hold a stored key or a real one in the environment; when neither source
    // holds a key, the run must refuse with the key's name. The env lock covers the read and the
    // removal, so a parallel secret test cannot flip the variable in between.
    let guard = crate::secret::ENV_LOCK.lock();
    let key_configured = std::env::var("OPENROUTER_API_KEY").map_or(false, |k| !k.trim().is_empty()) || crate::secret::has_key();
    if !key_configured {
        // SAFETY: the test holds the env lock; no parallel test reads the key
        unsafe { std::env::remove_var("OPENROUTER_API_KEY") };
        let error = run("run 2", &store, &worker()).await.unwrap_err();
        assert!(format!("{error:#}").contains("OPENROUTER_API_KEY") || format!("{error:#}").contains("key"), "{error:#}");
    }
    drop(guard);
}
