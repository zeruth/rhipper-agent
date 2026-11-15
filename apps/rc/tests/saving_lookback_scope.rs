use ra_console::retro;
use ra_roadmap::model::{NodeKind, TaskState};
use ra_roadmap::store::{Card, Store};
use ra_usage::{Entry, UsageStore};

fn row(node: i64) -> Entry {
    row_of(node, 100)
}

fn row_of(node: i64, tokens: u64) -> Entry {
    Entry {
        model: "claude-opus-5-5".to_owned(),
        role: None,
        node: Some(node),
        prompt_tokens: tokens,
        completion_tokens: 0,
        cached_tokens: 0,
        cost_usd: None,
        cost_computed: false,
        turns: 1,
        read_in: 0,
    }
}

/// The `[saving]` lines for a landed card claiming `claim`, after `later` cards ran after it, each
/// refused `refusals` commands; the cards before it were refused twice a run.
async fn scored(claim: &str, later: i64, refusals: usize) -> Vec<String> {
    scored_at(claim, later, refusals, 100).await
}

/// `scored` with the later cards' turns at `tokens` a turn against 100 before.
async fn scored_at(claim: &str, later: i64, refusals: usize, tokens: u64) -> Vec<String> {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    let card = Card { saving: Some(claim), ..Card::new() };
    let id = store.insert_card(Some(1), NodeKind::Task, "Landed", &card).await.unwrap();
    store.set_state(id, TaskState::Implemented).await.unwrap();
    let node = store.read(id).await.unwrap();
    let ledger = UsageStore::in_memory().await.unwrap();
    ledger.record(&row(11)).await.unwrap();
    let mark = ledger.last_id().await.unwrap();
    ledger.record(&row(id)).await.unwrap();
    let mut journal = "0\t11\tshell\tCommand\tgit stash\n".repeat(2);
    for other in 21..21 + later {
        ledger.record(&row_of(other, tokens)).await.unwrap();
        journal.push_str(&format!("0\t{other}\tshell\tCommand\tgit stash\n").repeat(refusals));
    }
    std::fs::create_dir_all(dir.path().join("var")).unwrap();
    std::fs::write(dir.path().join("var/gotchas.log"), journal).unwrap();
    retro::landed(dir.path(), &node, &ledger, mark).await
}

#[tokio::test]
async fn a_claim_on_refused_commands_is_pending_until_later_runs_exist() {
    let lines = scored("no refused command elsewhere", 1, 0).await;
    assert!(lines[0].contains("pending"), "{lines:?}");
    assert!(!lines[0].contains("missed") && !lines[0].contains("held"), "{lines:?}");
}

#[tokio::test]
async fn a_claim_on_reruns_is_pending_until_later_runs_exist() {
    let lines = scored("fewer whole-card reruns", 0, 0).await;
    assert!(lines[0].contains("pending"), "{lines:?}");
}

#[tokio::test]
async fn later_runs_with_fewer_refusals_hold_the_claim_and_more_miss_it() {
    let held = scored("no refused command elsewhere", 3, 0).await;
    assert!(held[0].contains("held: 0 refused commands in 3 later runs"), "{held:?}");
    let missed = scored("no refused command elsewhere", 3, 3).await;
    assert!(missed[0].contains("missed: 9 refused commands in 3 later runs"), "{missed:?}");
}

#[tokio::test]
async fn a_claim_on_evidence_line_tokens_waits_for_later_cards_then_compares_their_turns() {
    let pending = scored_at("fewer evidence-line tokens a turn", 2, 0, 60).await;
    assert!(pending[0].contains("pending: 2 of 3"), "{pending:?}");
    let held = scored_at("fewer evidence-line tokens a turn", 3, 0, 60).await;
    assert!(held[0].contains("held: 60 tokens/turn over 3 later cards vs median 100"), "{held:?}");
    let missed = scored_at("fewer evidence-line tokens a turn", 3, 0, 150).await;
    assert!(missed[0].contains("missed: 150 tokens/turn"), "{missed:?}");
}
