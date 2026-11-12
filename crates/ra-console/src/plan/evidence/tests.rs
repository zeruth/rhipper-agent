//! The tests of the evidence lines: a ledger row, a card run twice, another project's digest
//! and the gotcha journal's counts each become a cited line.

use ra_usage::{Entry, UsageStore};

use super::*;

#[test]
fn the_journals_counts_and_unmatched_commands_are_cited_by_gotcha_with_their_tally_and_an_empty_root_yields_none() {
    let root = tempfile::tempdir().unwrap();
    assert!(gotchas(root.path()).is_empty(), "no journal yet, no lines");
    let summary = Summary {
        counts: vec![("mojibake".to_owned(), 2)],
        unmatched: vec![("Get-ChildItem -Recurse".to_owned(), 3), ("Select-String foo".to_owned(), 1)],
    };
    assert_eq!(
        gotcha_lines(&summary).iter().map(ToString::to_string).collect::<Vec<_>>(),
        [
            "[gotchas.log mojibake] gotcha mojibake fired 2 times in the last day",
            "[gotchas.log unmatched-denied] refused command `Get-ChildItem -Recurse` matched no gotcha, 3 times in the last day",
            "[gotchas.log unmatched-denied] refused command `Select-String foo` matched no gotcha, once in the last day",
        ]
    );
}

/// A handoff that ended `outcome`, with `verdict` when a reviewer judged it.
fn run(outcome: &str, verdict: Option<&str>) -> Handoff {
    Handoff {
        when: 1_700_000_000,
        model: "claude-opus-5-5".to_owned(),
        outcome: outcome.to_owned(),
        check: "cargo test -p x · passed".to_owned(),
        verdict: verdict.map(str::to_owned),
        changed: vec!["src/lib.rs".to_owned()],
        report: "done".to_owned(),
        warnings: vec![],
        read: vec![],
    }
}

#[test]
fn a_card_run_twice_yields_its_line_with_its_runs_and_last_verdict_and_a_once_run_card_none() {
    let nodes = ra_roadmap::markdown::parse(
        "- 1 ‹milestone› M\n- 2 ‹task› Once {c2}\n    objective: a goal\n- 3 ‹task› Twice {c2}\n    objective: a goal\n- 4 ‹task› Never {c2}\n    objective: a goal\n",
    )
    .unwrap();
    let runs_of = |id: i64| match id {
        2 => vec![run("implemented", Some("accepted by glm"))],
        3 => vec![run("needs revision (trivial)", Some("needs revision (trivial) · 1 point(s) by glm")), run("implemented", None)],
        _ => vec![],
    };
    let lines = reruns_from(&nodes, runs_of);
    assert_eq!(
        lines.iter().map(ToString::to_string).collect::<Vec<_>>(),
        ["[context #3] card #3 Twice ran 2 times (needs revision (trivial), implemented), last verdict: no verdict yet"]
    );
    // the milestone is no card: its id never reaches the reader
    let lines = reruns_from(&nodes, |id| if id == 1 { vec![run("implemented", None); 2] } else { vec![] });
    assert!(lines.is_empty(), "{lines:?}");
}

#[test]
fn a_card_run_twice_in_its_context_file_is_read_from_the_root() {
    let dir = tempfile::tempdir().unwrap();
    let nodes = ra_roadmap::markdown::parse("- 2 ‹task› T {c2}\n    objective: a goal\n").unwrap();
    let mut task = TaskContext::open(dir.path(), 2);
    task.record(run("needs revision", None));
    task.record(run("implemented", Some("accepted by glm")));
    task.save(dir.path()).unwrap();
    let lines = reruns(dir.path(), &nodes);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].source, "context #2");
    assert!(lines[0].text.ends_with("ran 2 times (needs revision, implemented), last verdict: accepted by glm"), "{}", lines[0].text);
}

#[test]
fn a_hub_with_two_other_projects_digests_yields_two_lines_and_never_this_projects() {
    let hub = tempfile::tempdir().unwrap();
    let me = tempfile::tempdir().unwrap();
    let dir = hub.path().join("digest");
    let now = 1_700_000_000;
    // every digest names a directory that is on disk, so none is dropped as gone
    let digest = |name: &str, live: Option<&str>, last: Option<&str>| Digest {
        name: name.to_owned(),
        path: me.path().display().to_string(),
        pid: 1,
        when: now,
        live: live.map(str::to_owned),
        last: last.map(str::to_owned),
        ready: 3,
        changed: vec![],
        leftovers: vec![],
    };
    hub::write_digest_in(&dir, &digest("alpha", Some("#12 The digest · pid 4242 · started 10:04Z"), Some("✓ #11 Notes"))).unwrap();
    hub::write_digest_in(&dir, &digest("beta", None, Some("✓ #7 The ledger accepted"))).unwrap();
    hub::write_digest_in(&dir, &digest(&hub::project_name(me.path()), None, None)).unwrap();
    let lines = digests_from(&dir, me.path(), |_| true, now);
    let path = me.path().display();
    assert_eq!(
        lines.iter().map(ToString::to_string).collect::<Vec<_>>(),
        [
            format!("[hub digest/alpha.txt] alpha ({path}) · live: #12 The digest · pid 4242 · started 10:04Z"),
            format!("[hub digest/beta.txt] beta ({path}) · last: ✓ #7 The ledger accepted"),
        ]
    );
    assert_eq!(digest_line(&digest("gamma", None, None)).text, format!("gamma ({path}) · 3 ready, nothing run yet"));
}

#[test]
fn the_look_backs_of_a_project_and_the_hub_are_cited_by_file_and_line_in_order_and_an_empty_hub_yields_none() {
    use crate::retro;
    let hub = tempfile::tempdir().unwrap();
    let shared = hub.path().join("notes.log");
    let me = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    assert!(retro::evidence_with(me.path(), &shared).is_empty());
    // two runs here, one bullet of each about the machine, and the person's word
    retro::record_with(
        me.path(),
        &shared,
        Some(7),
        &retro::parse("## Faster next time\n- [machine] sed is refused\n- the card named no files\n"),
    )
    .unwrap();
    retro::record_with(me.path(), &shared, Some(8), &retro::parse("## Faster next time\n- the check ran twice\n")).unwrap();
    retro::record_pair(me.path(), "the tooltip is slow").unwrap();
    // another project's machine lesson, and one of this project's own that is not read back
    retro::record_with(other.path(), &shared, Some(3), &retro::parse("## Faster next time\n- [machine] rustfmt is refused\n")).unwrap();
    let lines = retro::evidence_with(me.path(), &shared);
    let name = other.path().file_name().unwrap().to_string_lossy();
    assert_eq!(
        lines.iter().map(ToString::to_string).collect::<Vec<_>>(),
        [
            "[retro.log:1] card #7: [machine] sed is refused".to_owned(),
            "[retro.log:2] card #7: the card named no files".to_owned(),
            "[retro.log:3] card #8: the check ran twice".to_owned(),
            "[retro.log:4] the person: the tooltip is slow".to_owned(),
            format!("[notes.log:2] ({name}) [machine] rustfmt is refused"),
        ]
    );
    // only the tail of a long log is read, its line numbers kept
    for n in 0..retro::EVIDENCE_LINES {
        retro::record_with(me.path(), &shared, Some(9), &retro::parse(&format!("## Faster next time\n- lesson {n}\n"))).unwrap();
    }
    let lines = retro::evidence_with(me.path(), &shared);
    assert_eq!(lines.len(), retro::EVIDENCE_LINES + 1);
    assert_eq!(lines[0].source, "retro.log:5");
    assert_eq!(lines[0].text, "card #9: lesson 0");
}

#[tokio::test]
async fn the_ledger_lines_name_the_card_its_cost_and_its_average_a_turn() {
    let store = UsageStore::in_memory().await.unwrap();
    let entry = |node: i64, turns: u64, prompt: u64, cost: Option<f64>| Entry {
        model: "claude-opus-5-5".to_owned(),
        role: None,
        node: Some(node),
        prompt_tokens: prompt,
        completion_tokens: 0,
        cached_tokens: 0,
        cost_usd: cost,
        cost_computed: false,
        turns,
        read_in: 0,
    };
    store.record(&entry(12, 8, 4_000, Some(0.4))).await.unwrap();
    store.record(&entry(12, 2, 1_000, Some(0.1))).await.unwrap();
    store.record(&entry(13, 1, 300, None)).await.unwrap();
    let lines = ledger(&store).await.unwrap();
    assert_eq!(
        lines.iter().map(ToString::to_string).collect::<Vec<_>>(),
        [
            "[usage.db #12] card #12 cost $0.50 in the last day over 2 runs and 10 turn(s), 500 tokens a turn on average",
            "[usage.db #13] card #13 cost an unreported amount in the last day over 1 run and 1 turn(s), 300 tokens a turn on average",
        ]
    );
}
