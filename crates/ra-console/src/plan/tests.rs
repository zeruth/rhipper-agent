//! The tests of the planner's call and its verdict: the brief numbers the evidence, an uncited
//! card is dropped with its reason, a well-formed outline of five is the plan, and three cards
//! are a refusal. Then `rc plan` itself: a dry run adds nothing, a run adds the cards with their
//! evidence and saving, and a walk after a cheaper day reads a saving back as held, once.

use std::fmt::Write;

use ra_context::{Handoff, TaskContext};
use ra_roadmap::model::{NodeKind, TaskState};
use ra_roadmap::store::{Card, Store};
use ra_usage::{Entry, UsageStore};

use crate::agent::Worker;
use crate::commands::plan::{self as command, DAY, Day, Measure, verdict};
use crate::commands::tests::{Scripted, says};

use super::evidence::Line;
use super::propose::{self, Refusal};

/// Five lines of evidence, numbered 1 to 5 in the brief.
fn evidence() -> Vec<Line> {
    (1..=5).map(|n| Line { source: format!("usage.db #{n}"), text: format!("card #{n} cost ${n}.00 in the last day") }).collect()
}

/// One card of an outline; an empty `evidence`, `saves` or `acceptance` leaves the field out.
fn card(title: &str, evidence: &str, saves: &str, acceptance: &str) -> String {
    let mut text = format!("- {title} {{c2}}\n");
    for (field, value) in [("evidence", evidence), ("saves", saves), ("acceptance", acceptance)] {
        if !value.is_empty() {
            let _ = writeln!(text, "  {field}: {value}");
        }
    }
    text.push_str("  objective: a goal\n  files: src/lib.rs\n");
    text
}

/// `count` well-formed cards, each citing its own line.
fn well_formed(count: usize) -> String {
    (1..=count).map(|n| card(&format!("Card {n}"), &format!("#{n}"), "turns", "`cargo test -p x` passes")).collect()
}

#[tokio::test]
async fn an_uncited_card_is_dropped_with_its_reason_and_the_rest_stand() {
    let dir = tempfile::tempdir().unwrap();
    let text = format!("{}{}", well_formed(4), card("Guess", "", "turns", "`cargo test -p x` passes"));
    let script = Scripted::new(vec![says(&text, vec![])]);
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(script.clone());
    let plan = propose::propose(dir.path(), &worker, &evidence()).await.unwrap();
    assert_eq!(plan.cards.iter().map(|c| c.item.title.as_str()).collect::<Vec<_>>(), ["Card 1", "Card 2", "Card 3", "Card 4"]);
    assert_eq!(plan.dropped.iter().map(ToString::to_string).collect::<Vec<_>>(), ["Guess: cites no evidence line"]);
    // the planner read the numbered brief under the instruction
    let seen = script.seen.lock();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].messages[1].content.contains("3. [usage.db #3] card #3 cost $3.00"), "{}", seen[0].messages[1].content);
}

#[tokio::test]
async fn a_well_formed_outline_of_five_is_kept_with_what_each_card_cites_and_saves() {
    let dir = tempfile::tempdir().unwrap();
    let text = format!("```\n{}```\n", well_formed(5));
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(Scripted::new(vec![says(&text, vec![])]));
    let plan = propose::propose(dir.path(), &worker, &evidence()).await.unwrap();
    assert_eq!(plan.cards.len(), 5, "{:?}", plan.dropped);
    assert!(plan.dropped.is_empty(), "{:?}", plan.dropped);
    let third = &plan.cards[2];
    assert_eq!((third.evidence.as_slice(), third.saves.as_str()), ([3].as_slice(), "turns"));
    assert_eq!(third.item.acceptance.as_deref(), Some("`cargo test -p x` passes"));
    assert_eq!((third.item.complexity, third.item.files.as_slice()), (2, ["src/lib.rs".to_owned()].as_slice()));
    assert_eq!(plan.model, "z-ai/glm-5.3-flash");
}

#[tokio::test]
async fn a_three_card_outline_is_refused_not_planned() {
    let dir = tempfile::tempdir().unwrap();
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(Scripted::new(vec![says(&well_formed(3), vec![])]));
    let error = propose::propose(dir.path(), &worker, &evidence()).await.unwrap_err();
    let refusal = error.downcast_ref::<Refusal>().unwrap_or_else(|| panic!("{error:#}"));
    assert_eq!(refusal.kept, 3);
    assert_eq!(format!("{error:#}"), "the planner's 3 card(s) are no plan: 4 to 8 must stand");
}

#[test]
fn the_verdict_names_every_reason_a_card_is_no_card() {
    let text = format!(
        "{}{}{}{}",
        card("Saves nothing", "#1", "", "`cargo test` passes"),
        card("Checks nothing", "#2", "tokens", "it works"),
        card("Cites past the brief", "#9", "tokens", "`cargo test` passes"),
        card("Cites two", "#1, #4", "a refused command", "`cargo test` passes and `cargo clippy` warns nothing")
    );
    let (cards, dropped) = propose::judge(&text, 5);
    assert_eq!(cards.iter().map(|c| (c.item.title.as_str(), c.evidence.clone())).collect::<Vec<_>>(), [("Cites two", vec![1, 4])]);
    assert_eq!(
        dropped.iter().map(ToString::to_string).collect::<Vec<_>>(),
        [
            "Saves nothing: names no saving",
            "Checks nothing: names no check in backticks",
            "Cites past the brief: cites evidence #9, the brief has 5 line(s)"
        ]
    );
    // nine cards that all stand are too many to be a plan
    let (cards, dropped) = propose::judge(&well_formed(9), 9);
    assert_eq!((cards.len(), dropped.len()), (9, 0));
}

/// A project with one open milestone and one look-back of the person's: the plan's only
/// evidence line of its own, numbered 1 ahead of whatever the test hub holds.
async fn planning_project() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    store.insert(Some(1), NodeKind::Milestone, "Phase N", None, None, 3).await.unwrap();
    crate::retro::record_pair(dir.path(), "the check ran twice").unwrap();
    (dir, store)
}

/// Four cards that each cite the first line; the last asks for more complexity than a planned
/// card gets.
fn cites_the_first() -> String {
    let mut text: String = (1..=3).map(|n| card(&format!("Card {n}"), "#1", "turns", "`cargo test -p x` passes")).collect();
    text.push_str(
        "- Big {c5}\n  evidence: #1, #1\n  saves: a refused command\n  acceptance: `cargo test -p x` passes\n  objective: a goal\n  files: src/lib.rs\n",
    );
    text
}

#[tokio::test]
async fn a_dry_run_shows_the_plan_with_what_each_card_cites_and_adds_nothing() {
    let (_dir, store) = planning_project().await;
    let script = Scripted::new(vec![says(&cites_the_first(), vec![])]);
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(script.clone());
    let out = command::plan(&store, &worker, None, true).await.unwrap();
    assert!(out[0].starts_with("4 card(s) from ") && out[0].contains(" by glm-5.3-flash · in 100 out 10 · $0.0100"), "{out:?}");
    assert!(out.iter().any(|l| l == "  Card 1 {c2} · evidence: retro.log:1 · saves: turns"), "{out:?}");
    assert!(out.iter().any(|l| l == "  Big {c3} · evidence: retro.log:1 · saves: a refused command"), "{out:?}");
    assert_eq!(out.last().unwrap(), "dry run: nothing added under #2 Phase N");
    assert_eq!(store.all().await.unwrap().len(), 2);
    // the planner read this project's look-back as the first line of the brief
    let seen = script.seen.lock();
    assert!(seen[0].messages[1].content.contains("1. [retro.log:1] the person: the check ran twice"), "{}", seen[0].messages[1].content);
}

#[tokio::test]
async fn a_run_adds_the_cards_under_the_last_open_milestone_with_their_evidence_and_saving() {
    let (dir, store) = planning_project().await;
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(Scripted::new(vec![says(&cites_the_first(), vec![])]));
    let out = command::plan(&store, &worker, None, false).await.unwrap();
    assert_eq!(out[1], "#3 Card 1 {c2} · evidence: retro.log:1 · saves: turns");
    assert_eq!(out[4], "#6 Big {c3} · evidence: retro.log:1 · saves: a refused command");
    assert_eq!(
        out.last().unwrap(),
        "added 4 planned card(s) under #2 Phase N by glm-5.3-flash · no turn cap: [worker] turns in rhipper.toml would hold each to one run"
    );
    // the cards are on the file with both fields, read back through a fresh store
    let reopened = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    let nodes = reopened.all().await.unwrap();
    assert_eq!(nodes.len(), 6);
    let big = &nodes[5];
    assert_eq!((big.id, big.parent, big.kind, big.complexity, big.state), (6, Some(2), NodeKind::Task, 3, TaskState::Defined));
    assert_eq!((big.evidence.as_deref(), big.saving.as_deref()), (Some("retro.log:1"), Some("a refused command")));
    assert_eq!((big.acceptance.as_deref(), big.files.as_slice()), (Some("`cargo test -p x` passes"), ["src/lib.rs".to_owned()].as_slice()));
    assert!(nodes[2..].iter().all(|n| n.evidence.is_some() && n.saving.is_some()), "{nodes:?}");
    // the pair feed heard of it, and a card is no place to plan under
    assert!(crate::pair::recent(dir.path(), 5).iter().any(|l| l.starts_with("added 4 planned card(s) under #2 Phase N")));
    let error = command::target(&nodes, Some(3)).unwrap_err();
    assert_eq!(format!("{error:#}"), "#3 is a task; plan under a milestone or a section");
}

#[test]
fn the_verdict_holds_when_the_last_day_is_under_the_day_before_and_waits_without_one() {
    let day = |turns, tokens_per_turn, refusals| Day { turns, tokens_per_turn, cost: None, refusals };
    assert_eq!(
        verdict("two turns a run", day(40, 0, 0), day(30, 0, 0)),
        Some((true, "claimed \"two turns a run\"; 40 turn(s) the day before, 30 turn(s) the last day".to_owned()))
    );
    assert_eq!(verdict("the person's intervention", day(30, 0, 0), day(30, 0, 0)).map(|v| v.0), Some(false));
    assert_eq!(verdict("a refused sed", day(0, 0, 3), day(0, 0, 3)).map(|v| v.0), Some(false));
    assert_eq!(verdict("a refused sed", day(0, 0, 0), day(0, 0, 0)).map(|v| v.0), Some(true));
    // nothing ran the day before: the claim waits
    assert_eq!(verdict("fewer turns", day(0, 0, 0), day(5, 0, 0)), None);
    assert_eq!((Measure::of("$0.30 a card"), Measure::of("a smaller context")), (Measure::Cost, Measure::Tokens));
    // a cost claim without a reported cost falls to the tokens a turn
    let cheaper = verdict("cheaper runs", day(10, 900, 0), day(10, 300, 0)).unwrap();
    assert_eq!(cheaper, (true, "claimed \"cheaper runs\"; 900 tokens a turn the day before, 300 tokens a turn the last day".to_owned()));
}

#[tokio::test]
async fn a_walk_after_a_cheaper_day_records_that_the_saving_held_once() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    let card = Card { saving: Some("fewer tokens a turn"), evidence: Some("usage.db #9"), ..Card::new() };
    let id = store.insert_card(Some(1), NodeKind::Task, "Smaller reads", &card).await.unwrap();
    store.set_state(id, TaskState::Accepted).await.unwrap();
    // its run ended more than a day ago
    let mut task = TaskContext::open(dir.path(), id);
    task.record(Handoff {
        when: crate::agent::unix_now() - DAY.as_secs() - 60,
        model: "claude-opus-5-5".to_owned(),
        outcome: "implemented".to_owned(),
        check: "cargo test -p x · passed".to_owned(),
        verdict: Some("accepted by glm".to_owned()),
        changed: vec!["src/lib.rs".to_owned()],
        report: "done".to_owned(),
        warnings: vec![],
        read: vec![],
    });
    task.save(dir.path()).unwrap();
    // the day before carried 1000 tokens a turn, the last day 200
    let ledger = UsageStore::in_memory().await.unwrap();
    let entry = |prompt: u64| Entry {
        model: "claude-opus-5-5".to_owned(),
        role: None,
        node: Some(id),
        prompt_tokens: prompt,
        completion_tokens: 0,
        cached_tokens: 0,
        cost_usd: None,
        cost_computed: false,
        turns: 2,
        read_in: 0,
    };
    ledger.record_ago(&entry(2_000), DAY + std::time::Duration::from_hours(1)).await.unwrap();
    ledger.record(&entry(400)).await.unwrap();
    let worker = Worker::new("z-ai/glm-5.3-flash").with_provider(Scripted::new(vec![])).with_ledger(ledger);
    let out = crate::agent::run_all(&store, &worker, false).await.unwrap();
    let held =
        format!("[saving] #{id} held: claimed \"fewer tokens a turn\"; 1000 tokens a turn the day before, 200 tokens a turn the last day");
    assert_eq!(out, vec![held.clone(), "ran 0 card(s) · no card is ready".to_owned()]);
    let notes = crate::retro::recent(dir.path(), 5);
    assert_eq!((notes.len(), notes[0].node, notes[0].text.as_str()), (1, Some(id), held.as_str()));
    // a second walk reads nothing back: the claim has its verdict
    let out = crate::agent::run_all(&store, &worker, false).await.unwrap();
    assert_eq!(out, vec!["ran 0 card(s) · no card is ready"]);
    assert_eq!(crate::retro::recent(dir.path(), 5).len(), 1);
    // the next plan reads the verdict as evidence; a worker's machine notes leave it out
    assert!(crate::retro::evidence(dir.path()).iter().any(|l| l.text.contains("[saving]")));
    assert!(!crate::retro::machine_lines(dir.path()).contains("[saving]"));
}
