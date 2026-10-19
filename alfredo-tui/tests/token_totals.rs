//! Token and model-time totals: the plan receipt keeps the planner's counts and
//! `/go` sums them with the workers' for its own tasks.
use alfredo_tui::metrics::{Metrics, Usage};

fn metrics(prompt: Option<u64>, eval: Option<u64>, total_ns: Option<u64>) -> Metrics {
    Metrics {
        prompt_eval_count: prompt,
        eval_count: eval,
        total_duration: total_ns,
        load_duration: Some(7),
        ..Metrics::default()
    }
}

#[test]
fn usage_keeps_only_counts_and_total_time() {
    let usage = Usage::from_metrics(&metrics(Some(709), Some(95), Some(2_000_000_000))).unwrap();
    assert_eq!(
        (usage.prompt_tokens, usage.completion_tokens, usage.model_ns),
        (Some(709), Some(95), Some(2_000_000_000))
    );
    // A server that reports no counts and no time leaves nothing to keep.
    assert_eq!(Usage::from_metrics(&Metrics::default()), None);
}

#[test]
fn usage_serialises_compactly_and_old_receipts_without_it_still_load() {
    let usage = Usage::from_metrics(&metrics(Some(1), None, None)).unwrap();
    assert_eq!(
        serde_json::to_string(&usage).unwrap(),
        r#"{"prompt_tokens":1}"#
    );
    let full =
        Usage::from_metrics(&metrics(Some(12_300), Some(1_100), Some(38_000_000_000))).unwrap();
    assert!(serde_json::to_vec(&full).unwrap().len() <= 96);
    assert_eq!(
        serde_json::from_str::<Usage>("{}").unwrap(),
        Usage::default()
    );
}

#[test]
fn totals_line_is_bounded_and_omitted_without_token_counts() {
    let mut total = Usage::default();
    assert_eq!(total.line(), None);
    total.add(
        &Usage::from_metrics(&metrics(Some(11_000), Some(100), Some(30_000_000_000))).unwrap(),
    );
    total.add(
        &Usage::from_metrics(&metrics(Some(1_300), Some(1_000), Some(8_400_000_000))).unwrap(),
    );
    assert_eq!(
        total.line().unwrap(),
        "tokens 12.3k in / 1.1k out · model 38 s"
    );
    let mut small = Usage::default();
    small.add(&Usage::from_metrics(&metrics(Some(900), Some(40), Some(400_000_000))).unwrap());
    assert_eq!(small.line().unwrap(), "tokens 900 in / 40 out · model <1 s");
    // Time alone is not a token total.
    let mut timed = Usage::default();
    timed.add(&Usage::from_metrics(&metrics(None, None, Some(5_000_000_000))).unwrap());
    assert_eq!(timed.line(), None);
}
