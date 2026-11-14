//! A card whose last run ended `implemented`, with its branch head and its notes as that run left
//! them, goes to review and starts no worker; a new commit or a note after the run starts one again.

use std::path::Path;
use std::process::Command;

use ra_context::{Handoff, TaskContext};
use ra_roadmap::history::Source;
use ra_roadmap::model::{NodeKind, TaskState};

/// The card's id: the project is #1 and the card the first node under it.
const CARD: i64 = 2;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").args(args).current_dir(dir).output().expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn commit(dir: &Path, name: &str) {
    std::fs::write(dir.join(name), name).expect("a file");
    git(dir, &["add", "."]);
    git(dir, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", name]);
}

fn handoff(outcome: &str, report: String) -> Handoff {
    Handoff {
        when: 1,
        model: "m".to_owned(),
        outcome: outcome.to_owned(),
        check: String::new(),
        verdict: None,
        changed: Vec::new(),
        report,
        warnings: Vec::new(),
        read: Vec::new(),
        failures: Vec::new(),
    }
}

/// A git project with one card left implemented by a run whose handoff names the head.
async fn project() -> (tempfile::TempDir, ra_roadmap::store::Store) {
    let dir = tempfile::tempdir().expect("a tempdir");
    git(dir.path(), &["init", "-q"]);
    commit(dir.path(), "a.txt");
    let store = ra_roadmap::store::Store::open(&dir.path().join("rhipper.md"), "Rhipper Agent").await.expect("the store");
    let id = store.insert(Some(1), NodeKind::Task, "Make", None, None, 3).await.expect("a card");
    assert_eq!(id, CARD);
    store.set_state_by(id, TaskState::Implemented, Source::Run).await.expect("the state");
    let tip = git(dir.path(), &["rev-parse", "--short=12", "HEAD"]);
    let mut task = TaskContext::open(dir.path(), id);
    task.record(handoff("implemented", format!("tree: at {tip}\ndone")));
    task.save(dir.path()).expect("the handoff saves");
    (dir, store)
}

#[tokio::test]
async fn an_implemented_card_with_nothing_new_is_not_rerun() {
    let (_dir, store) = project().await;
    assert!(ra_console::agent::reported_implemented(&store, CARD).await.expect("the card reads"));
}

#[tokio::test]
async fn a_new_commit_since_the_run_starts_a_worker_again() {
    let (dir, store) = project().await;
    commit(dir.path(), "b.txt");
    assert!(!ra_console::agent::reported_implemented(&store, CARD).await.expect("the card reads"));
}

#[tokio::test]
async fn a_note_after_the_run_starts_a_worker_again() {
    let (dir, store) = project().await;
    let mut task = TaskContext::open(dir.path(), CARD);
    task.record(handoff("note from #4", "look again".to_owned()));
    task.save(dir.path()).expect("the note saves");
    assert!(!ra_console::agent::reported_implemented(&store, CARD).await.expect("the card reads"));
}

#[tokio::test]
async fn a_card_sent_back_is_not_reported_implemented() {
    let (_dir, store) = project().await;
    store.set_state_by(CARD, TaskState::NeedsRevision, Source::Run).await.expect("the state");
    assert!(!ra_console::agent::reported_implemented(&store, CARD).await.expect("the card reads"));
}
