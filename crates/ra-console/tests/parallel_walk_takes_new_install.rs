use ra_console::agent::{Worker, handover_line, run_parallel, walk_running};
use ra_roadmap::store::Store;

/// The relaunch the tests hand in: it starts nothing and names a pid.
fn relaunched(_dir: &std::path::Path, _root: &std::path::Path) -> anyhow::Result<u32> {
    Ok(4242)
}

/// The relaunch of an install that cannot start.
fn cannot_start(_dir: &std::path::Path, _root: &std::path::Path) -> anyhow::Result<u32> {
    anyhow::bail!("no rc there")
}

/// A worker whose walks watch `install` and start again through `relaunch`.
fn walker(install: &std::path::Path, relaunch: fn(&std::path::Path, &std::path::Path) -> anyhow::Result<u32>) -> Worker {
    Worker::new("z-ai/glm-5.3-flash").with_install(Some(install.to_path_buf())).with_relaunch(relaunch)
}

#[tokio::test]
async fn a_parallel_walk_hands_over_to_a_newer_installed_rc_and_lets_its_lock_go() {
    let install = tempfile::tempdir().unwrap();
    std::fs::write(install.path().join(ra_console::hub::INSTALLED_FILE), "99.0.0 1\n").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    let out = run_parallel(&store, &walker(install.path(), relaunched), 4, false).await.unwrap();
    assert_eq!(out.last().unwrap(), "moved to 99.0.0 · ran 0 card(s) · pid 4242", "{out:?}");
    let feed = std::fs::read_to_string(dir.path().join("var/pair.log")).unwrap();
    assert!(feed.contains("walk moving to 99.0.0: it starts no new card"), "{feed}");
    assert!(feed.contains("walk moved to 99.0.0"), "{feed}");
    // the lock went with the walk: the successor takes it
    assert!(!walk_running(dir.path()));
}

#[tokio::test]
async fn a_parallel_walk_whose_new_rc_cannot_start_stays_on_its_own_and_goes_on() {
    let install = tempfile::tempdir().unwrap();
    std::fs::write(install.path().join(ra_console::hub::INSTALLED_FILE), "99.0.0 1\n").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    let out = run_parallel(&store, &walker(install.path(), cannot_start), 4, false).await.unwrap();
    assert!(out.iter().any(|l| l.starts_with(&format!("the walk stays on {}", ra_console::VERSION))), "{out:?}");
    assert_eq!(out.last().unwrap(), "ran 0 card(s) · no card is ready", "{out:?}");
}

#[tokio::test]
async fn a_parallel_walk_on_the_installed_rc_does_not_move() {
    let install = tempfile::tempdir().unwrap();
    std::fs::write(install.path().join(ra_console::hub::INSTALLED_FILE), format!("{} 1\n", ra_console::VERSION)).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    let out = run_parallel(&store, &walker(install.path(), relaunched), 4, false).await.unwrap();
    assert_eq!(out.last().unwrap(), "ran 0 card(s) · no card is ready", "{out:?}");
}

#[test]
fn the_feed_says_the_walk_waits_for_its_runs_in_flight_before_it_hands_over() {
    assert_eq!(handover_line("1.2.3", 2), "walk moving to 1.2.3: it starts no new card and hands over once its 2 run(s) in flight end");
    assert_eq!(handover_line("1.2.3", 0), "walk moving to 1.2.3: it starts no new card and hands over now");
}
