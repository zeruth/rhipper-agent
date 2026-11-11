//! The tests of the ledger: the rows go in, the sums come out.

use super::*;

fn entry(model: &str, node: Option<i64>, cost: Option<f64>) -> Entry {
    Entry { model: model.to_owned(), node, prompt_tokens: 100, completion_tokens: 10, cached_tokens: 40, cost_usd: cost, turns: 1 }
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

#[test]
fn a_running_tally_matches_the_store() {
    let mut tally = Totals::default();
    tally.add(&entry("m", None, None));
    assert_eq!(tally.cost_usd, None);
    tally.add(&entry("m", None, Some(0.1)));
    assert_eq!(tally.calls, 2);
    assert_eq!(tally.cost_usd, Some(0.1));
}
