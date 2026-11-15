use ra_console::retro;
use ra_roadmap::model::{NodeKind, TaskState};
use ra_roadmap::store::{Card, Store};
use ra_usage::{Entry, UsageStore};

fn row(node: i64, turns: u64, prompt_tokens: u64) -> Entry {
    Entry {
        model: "claude-opus-5-5".to_owned(),
        role: None,
        node: Some(node),
        prompt_tokens,
        completion_tokens: 0,
        cached_tokens: 0,
        cost_usd: None,
        cost_computed: false,
        turns,
        read_in: 0,
    }
}

/// The `[saving]` lines the walk writes for a landed card claiming `saving`, its run 3 turns at
/// 110 tokens a turn against three cards before it at a median of 1000, with `refusals` refused
/// commands in the gotcha journal.
async fn scored(saving: &str, refusals: usize) -> Vec<String> {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    let card = Card { saving: Some(saving), ..Card::new() };
    let id = store.insert_card(Some(1), NodeKind::Task, "Landed", &card).await.unwrap();
    store.set_state(id, TaskState::Implemented).await.unwrap();
    let node = store.read(id).await.unwrap();
    let ledger = UsageStore::in_memory().await.unwrap();
    for (other, turns, tokens) in [(11, 2, 600), (12, 4, 4000), (13, 1, 5000)] {
        ledger.record(&row(other, turns, tokens)).await.unwrap();
    }
    let mark = ledger.last_id().await.unwrap();
    ledger.record(&row(id, 3, 330)).await.unwrap();
    if refusals > 0 {
        std::fs::create_dir_all(dir.path().join("var")).unwrap();
        let journal: String = (0..refusals).map(|_| format!("0\t{id}\tshell\tCommand\tgit stash\n")).collect();
        std::fs::write(dir.path().join("var/gotchas.log"), journal).unwrap();
    }
    retro::landed(dir.path(), &node, &ledger, mark).await
}

#[tokio::test]
async fn a_claim_on_refused_commands_waits_for_later_runs() {
    // its own run says nothing of refusals elsewhere
    let lines = scored("no refused command", 2).await;
    assert!(lines[0].contains("pending"), "{lines:?}");
}

#[tokio::test]
async fn a_claim_on_the_persons_intervention_counts_refusals_and_send_backs() {
    let held = scored("no intervention from the person", 0).await;
    assert!(held[0].contains("held: 0 interventions in the run"), "{held:?}");
    let missed = scored("no send-back for the person", 1).await;
    assert!(missed[0].contains("missed: 1 interventions in the run"), "{missed:?}");
}

#[tokio::test]
async fn a_claim_on_turns_or_tokens_is_scored_against_the_median() {
    let lines = scored("fewer tokens a turn", 3).await;
    assert!(lines[0].contains("held: 3 turns, 110 tokens/turn vs median 1000"), "{lines:?}");
}

#[tokio::test]
async fn a_claim_naming_no_unit_is_logged_unscored() {
    let lines = scored("a clearer verdict", 0).await;
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("unscored: claimed \"a clearer verdict\""), "{lines:?}");
    assert!(!lines[0].contains("missed") && !lines[0].contains("held"));
}
