//! The tests of the ledger: the rows go in, the sums come out.

use super::*;

fn entry(model: &str, node: Option<i64>, cost: Option<f64>) -> Entry {
    Entry { model: model.to_owned(), node, prompt_tokens: 100, completion_tokens: 10, cached_tokens: 40, cost_usd: cost }
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
