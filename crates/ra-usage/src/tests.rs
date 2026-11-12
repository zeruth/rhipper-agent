//! The tests of the ledger: the rows go in, the sums come out.

use super::*;

fn entry(model: &str, node: Option<i64>, cost: Option<f64>) -> Entry {
    Entry {
        model: model.to_owned(),
        role: None,
        node,
        prompt_tokens: 100,
        completion_tokens: 10,
        cached_tokens: 40,
        cost_usd: cost,
        cost_computed: false,
        turns: 1,
        read_in: 0,
    }
}

#[tokio::test]
async fn the_read_in_is_kept_per_call_and_summed_with_the_rest() {
    let store = UsageStore::in_memory().await.unwrap();
    store.record(&Entry { read_in: 700, ..entry("claude-opus-5-5", Some(3), Some(0.25)) }).await.unwrap();
    store.record(&Entry { read_in: 300, ..entry("claude-opus-5-5", Some(3), Some(0.25)) }).await.unwrap();
    store.record(&Entry { role: Some("summary".to_owned()), ..entry("local/qwen3-8b", Some(3), None) }).await.unwrap();
    let calls = store.calls_of(Some(3)).await.unwrap();
    assert_eq!(calls.iter().map(|c| c.read_in).collect::<Vec<_>>(), [0, 300, 700]);
    assert_eq!(store.totals().await.unwrap().read_in, 1000);
    let by_node = store.by_node().await.unwrap();
    assert_eq!(by_node[0].1.read_in, 1000);
}

#[tokio::test]
async fn a_computed_cost_is_marked_as_such_and_a_reported_one_is_not() {
    let store = UsageStore::in_memory().await.unwrap();
    store.record(&entry("claude-opus-5-5", Some(3), Some(0.25))).await.unwrap();
    store.record(&Entry { cost_computed: true, ..entry("z-ai/glm-5.3-flash", Some(3), Some(0.002)) }).await.unwrap();
    let calls = store.calls_of(Some(3)).await.unwrap();
    assert_eq!(calls.iter().map(|c| (c.cost_usd, c.cost_computed)).collect::<Vec<_>>(), [(Some(0.002), true), (Some(0.25), false)]);
    // the sums do not tell the two apart: the computed cost is the best figure there is
    assert_eq!(store.totals().await.unwrap().cost_usd, Some(0.252));
}

#[tokio::test]
async fn the_sums_carry_reported_and_computed_costs_in_one_figure_and_mark_the_estimated_share() {
    let store = UsageStore::in_memory().await.unwrap();
    store.record(&entry("claude-opus-5-5", Some(3), Some(3.0))).await.unwrap();
    store.record(&Entry { cost_computed: true, ..entry("claude-opus-5-5", Some(3), Some(1.12)) }).await.unwrap();
    store.record(&entry("local/qwen3-8b", Some(4), None)).await.unwrap();
    let totals = store.totals().await.unwrap();
    assert!((totals.estimated_usd - 1.12).abs() < 1e-9);
    assert_eq!(totals.cost_text(), "$4.1200 ($1.12 est.)");
    let by_node = store.by_node().await.unwrap();
    let mut merged = Totals::default();
    for (_, t) in &by_node {
        merged.merge(t);
    }
    assert_eq!(merged.cost_text(), "$4.1200 ($1.12 est.)");
    let mut added = Totals::default();
    added.add(&entry("claude-opus-5-5", None, Some(0.5)));
    assert_eq!(added.cost_text(), "$0.5000");
    assert_eq!(Totals::default().cost_text(), "—");
}

#[tokio::test]
async fn a_row_carries_its_turn_count_and_the_calls_of_a_node_list_it() {
    let store = UsageStore::in_memory().await.unwrap();
    store.record(&Entry { turns: 12, ..entry("claude-opus-5-5", Some(3), Some(0.25)) }).await.unwrap();
    store.record(&entry("z-ai/glm-5.3-flash", Some(3), None)).await.unwrap();
    store.record(&entry("claude-opus-5-5", None, Some(0.1))).await.unwrap();
    assert_eq!(store.totals().await.unwrap().turns, 14);
    // the latest call first, each with its own turns
    let calls = store.calls_of(Some(3)).await.unwrap();
    assert_eq!(calls.iter().map(|c| (c.model.as_str(), c.turns)).collect::<Vec<_>>(), [("z-ai/glm-5.3-flash", 1), ("claude-opus-5-5", 12)]);
    assert_eq!(calls[1].prompt_tokens, 100);
    assert_eq!(calls[1].cost_usd, Some(0.25));
    assert_eq!(store.calls_of(None).await.unwrap().len(), 1);
    assert!(store.calls_of(Some(9)).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_roles_call_carries_its_tag_and_the_rows_group_by_model_and_role() {
    let store = UsageStore::in_memory().await.unwrap();
    store.record(&Entry { turns: 6, ..entry("claude-opus-5-5", Some(3), Some(0.25)) }).await.unwrap();
    for role in ["summary", "classify", "digest"] {
        store.record(&Entry { role: Some(role.to_owned()), ..entry("local/qwen3-8b", Some(3), None) }).await.unwrap();
    }
    store.record(&Entry { role: Some("summary".to_owned()), ..entry("local/qwen3-8b", Some(4), None) }).await.unwrap();
    // the calls of the card name their role, the worker's none
    let calls = store.calls_of(Some(3)).await.unwrap();
    assert_eq!(
        calls.iter().map(|c| (c.model.as_str(), c.role.as_deref())).collect::<Vec<_>>(),
        [
            ("local/qwen3-8b", Some("digest")),
            ("local/qwen3-8b", Some("classify")),
            ("local/qwen3-8b", Some("summary")),
            ("claude-opus-5-5", None)
        ]
    );
    // by role: the worker's row, then each role's, the two summaries as one
    let by_role = store.by_role().await.unwrap();
    let rows: Vec<(&str, Option<&str>, u64)> = by_role.iter().map(|(m, r, t)| (m.as_str(), r.as_deref(), t.calls)).collect();
    assert_eq!(rows[0], ("claude-opus-5-5", None, 1));
    assert!(rows.contains(&("local/qwen3-8b", Some("summary"), 2)));
    assert!(rows.contains(&("local/qwen3-8b", Some("digest"), 1)));
    assert_eq!(rows.len(), 4);
    // by model does not tell the roles apart
    assert_eq!(store.by_model().await.unwrap().iter().find(|(m, _)| m == "local/qwen3-8b").unwrap().1.calls, 4);
}

#[tokio::test]
async fn an_empty_ledger_sums_to_nothing() {
    let store = UsageStore::in_memory().await.unwrap();
    assert_eq!(store.totals().await.unwrap(), Totals::default());
    assert!(store.by_model().await.unwrap().is_empty());
}

#[tokio::test]
async fn the_totals_sum_the_rows_and_keep_an_unknown_cost_unknown() {
    let store = UsageStore::in_memory().await.unwrap();
    store.record(&entry("claude-opus-5-5", Some(3), Some(0.25))).await.unwrap();
    store.record(&entry("claude-opus-5-5", None, Some(0.5))).await.unwrap();
    store.record(&entry("z-ai/glm-5.3-flash", Some(4), None)).await.unwrap();
    let totals = store.totals().await.unwrap();
    assert_eq!(totals.calls, 3);
    assert_eq!(totals.prompt_tokens, 300);
    assert_eq!(totals.completion_tokens, 30);
    assert_eq!(totals.cached_tokens, 120);
    assert_eq!(totals.cost_usd, Some(0.75));

    let by_model = store.by_model().await.unwrap();
    assert_eq!(by_model[0].0, "claude-opus-5-5");
    assert_eq!(by_model[0].1.calls, 2);
    assert_eq!(by_model[1].0, "z-ai/glm-5.3-flash");
    assert_eq!(by_model[1].1.cost_usd, None);

    // by node: the rows that served no node come first, as None
    let by_node = store.by_node().await.unwrap();
    assert_eq!(by_node.iter().map(|(n, _)| *n).collect::<Vec<_>>(), [None, Some(3), Some(4)]);
    assert_eq!(by_node[1].1.cost_usd, Some(0.25));
    let mut sum = Totals::default();
    for (_, t) in &by_node {
        sum.merge(t);
    }
    assert_eq!(sum, totals);
}

#[tokio::test]
async fn the_rows_after_a_mark_read_oldest_first_and_sum_apart() {
    let store = UsageStore::in_memory().await.unwrap();
    assert_eq!(store.last_id().await.unwrap(), 0);
    store.record(&entry("m", Some(1), Some(0.1))).await.unwrap();
    store.record(&Entry { prompt_tokens: 1_000, ..entry("m", Some(2), Some(0.2)) }).await.unwrap();
    let mark = store.last_id().await.unwrap();
    assert_eq!(mark, 2);
    store.record(&Entry { completion_tokens: 50, ..entry("m", Some(3), None) }).await.unwrap();
    store.record(&Entry { prompt_tokens: 300, ..entry("m", Some(3), Some(0.3)) }).await.unwrap();
    // since the mark: the two later rows, oldest first; the cap keeps the newest
    let since = store.recent(mark, 10).await.unwrap();
    assert_eq!(since.iter().map(|s| (s.tokens, s.cost_usd)).collect::<Vec<_>>(), [(150, None), (310, Some(0.3))]);
    assert_eq!(store.recent(0, 3).await.unwrap().iter().map(|s| s.tokens).collect::<Vec<_>>(), [1_010, 150, 310]);
    let after = store.totals_after(mark).await.unwrap();
    assert_eq!((after.calls, after.tokens(), after.cost_usd), (2, 460, Some(0.3)));
    assert_eq!(store.totals_after(0).await.unwrap(), store.totals().await.unwrap());
}

#[tokio::test]
async fn a_card_costs_the_sum_of_its_runs_and_averages_its_tokens_a_turn() {
    let store = UsageStore::in_memory().await.unwrap();
    // two runs on card 3: one of 12 turns, one of 4; a run on another card and a chat with none
    store
        .record(&Entry { turns: 12, prompt_tokens: 1_000, completion_tokens: 200, ..entry("claude-opus-5-5", Some(3), Some(0.25)) })
        .await
        .unwrap();
    store
        .record(&Entry { turns: 4, prompt_tokens: 300, completion_tokens: 100, ..entry("z-ai/glm-5.3-flash", Some(3), None) })
        .await
        .unwrap();
    store.record(&Entry { turns: 2, ..entry("claude-opus-5-5", Some(4), Some(0.1)) }).await.unwrap();
    store.record(&entry("claude-opus-5-5", None, Some(0.1))).await.unwrap();
    let card = store.card_cost(3).await.unwrap();
    assert_eq!((card.node, card.totals.calls, card.totals.turns, card.tokens()), (3, 2, 16, 1_600));
    assert_eq!(card.totals.cost_usd, Some(0.25));
    assert_eq!(card.tokens_per_turn(), 100);
    // every card at once, the chat left out
    let all = store.card_costs().await.unwrap();
    assert_eq!(all.iter().map(|c| (c.node, c.totals.turns, c.tokens_per_turn())).collect::<Vec<_>>(), [(3, 16, 100), (4, 2, 55)]);
    assert_eq!(all[0], card);
}

#[tokio::test]
async fn the_costliest_cards_of_the_day_rank_by_cost_and_average_their_tokens_a_turn() {
    let store = UsageStore::in_memory().await.unwrap();
    // three cards: 5 costs the most over two runs, 6 the least, 7 reported no cost at all
    store
        .record(&Entry { turns: 4, prompt_tokens: 700, completion_tokens: 100, ..entry("claude-opus-5-5", Some(5), Some(0.30)) })
        .await
        .unwrap();
    store
        .record(&Entry { turns: 6, prompt_tokens: 1_000, completion_tokens: 200, ..entry("claude-opus-5-5", Some(5), Some(0.50)) })
        .await
        .unwrap();
    store
        .record(&Entry { turns: 2, prompt_tokens: 100, completion_tokens: 20, ..entry("z-ai/glm-5.3-flash", Some(6), Some(0.01)) })
        .await
        .unwrap();
    store.record(&Entry { turns: 3, prompt_tokens: 300, completion_tokens: 30, ..entry("local/qwen", Some(7), None) }).await.unwrap();
    store.record(&entry("claude-opus-5-5", None, Some(9.0))).await.unwrap();
    // a run on card 8 from before the window, backdated as the ledger would have written it
    store.record(&Entry { turns: 1, ..entry("claude-opus-5-5", Some(8), Some(5.0)) }).await.unwrap();
    sqlx::query("update usage set at = datetime('now', '-2 days') where node = 8").execute(&store.pool).await.unwrap();
    let cards = store.costliest_cards(LAST_DAY, 10).await.unwrap();
    assert_eq!(
        cards.iter().map(|c| (c.node, c.totals.cost_usd, c.totals.turns, c.tokens_per_turn())).collect::<Vec<_>>(),
        [(5, Some(0.8), 10, 200), (6, Some(0.01), 2, 60), (7, None, 3, 110)]
    );
    // the limit keeps the top of the ranking
    assert_eq!(store.costliest_cards(LAST_DAY, 1).await.unwrap().iter().map(|c| c.node).collect::<Vec<_>>(), [5]);
}

#[tokio::test]
async fn the_recent_cards_up_to_a_mark_come_newest_first_and_sum_their_rows_in_it() {
    let store = UsageStore::in_memory().await.unwrap();
    // card 3 ran first, then 4, then 3 again; a chat in between; card 5 after the mark
    store.record(&Entry { turns: 2, ..entry("claude-opus-5-5", Some(3), None) }).await.unwrap();
    store.record(&Entry { turns: 4, ..entry("claude-opus-5-5", Some(4), None) }).await.unwrap();
    store.record(&entry("claude-opus-5-5", None, None)).await.unwrap();
    store.record(&Entry { turns: 3, ..entry("claude-opus-5-5", Some(3), None) }).await.unwrap();
    let mark = store.last_id().await.unwrap();
    store.record(&Entry { turns: 9, ..entry("claude-opus-5-5", Some(5), None) }).await.unwrap();
    let cards = store.recent_cards(mark, 20).await.unwrap();
    assert_eq!(cards.iter().map(|c| (c.node, c.totals.calls, c.totals.turns)).collect::<Vec<_>>(), [(3, 2, 5), (4, 1, 4)]);
    // the limit keeps the newest; a mark before a card's second run sums only its first
    assert_eq!(store.recent_cards(mark, 1).await.unwrap().iter().map(|c| c.node).collect::<Vec<_>>(), [3]);
    assert_eq!(store.recent_cards(2, 20).await.unwrap().iter().map(|c| (c.node, c.totals.turns)).collect::<Vec<_>>(), [(4, 4), (3, 2)]);
}

#[tokio::test]
async fn a_card_the_ledger_has_not_seen_costs_nothing() {
    let store = UsageStore::in_memory().await.unwrap();
    let card = store.card_cost(9).await.unwrap();
    assert_eq!(card, CardCost { node: 9, totals: Totals::default() });
    assert_eq!((card.tokens_per_turn(), card.totals.cost_usd), (0, None));
    assert!(store.card_costs().await.unwrap().is_empty());
}

#[test]
fn a_running_tally_matches_the_store() {
    let mut tally = Totals::default();
    tally.add(&entry("m", None, None));
    assert_eq!(tally.cost_usd, None);
    tally.add(&entry("m", None, Some(0.1)));
    assert_eq!(tally.calls, 2);
    assert_eq!(tally.cost_usd, Some(0.1));
}
