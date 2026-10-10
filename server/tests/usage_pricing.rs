//! Isolated aggregate/price regressions, using synthetic metadata only.
use cursor_server::store::{ModelPricingSettings, ModelTokenPrice, Store};

fn ms(value: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(value)
        .unwrap()
        .timestamp_millis()
}

async fn call(store: &Store, id: &str, model: &str, kind: &str, at: i64, tokens: [i64; 4]) {
    sqlx::query("INSERT INTO llm_calls(call_id, run_id, conversation_id, provider_call_index, model_hash, provider_type, provider_url, request_type, request_url, model_id, display_name, status, created_at_ms, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, message_count, tool_count, detailed) VALUES (?, 'synthetic-run', 'synthetic-conversation', 0, ?, ?, 'https://example.invalid', ?, 'https://example.invalid/v1', ?, ?, 'completed', ?, ?, ?, ?, ?, 0, 0, 0)")
        .bind(id).bind(model).bind(kind).bind(kind).bind(model).bind(model).bind(at)
        .bind(tokens[0]).bind(tokens[1]).bind(tokens[2]).bind(tokens[3])
        .execute(store.pool()).await.unwrap();
}

#[tokio::test]
async fn local_day_multi_model_and_prices_share_the_same_cache_denominator() {
    let temp = tempfile::tempdir().unwrap();
    let path = format!("sqlite://{}", temp.path().join("usage.db").display());
    let store = Store::connect(&path).await.unwrap();
    let start = ms("2026-03-08T08:00:00Z");
    let end = ms("2026-03-09T07:00:00Z");
    call(&store, "openai", "a", "openai", start, [1000, 10, 900, 50]).await;
    call(
        &store,
        "anthropic",
        "b",
        "anthropic",
        end - 1,
        [100, 20, 100, 50],
    )
    .await;
    call(&store, "outside", "a", "openai", end, [99000, 0, 0, 0]).await;
    let filter = Some("[\"a\",\"b\"]");
    let overview = store
        .overview(Some(start), Some(end), filter, None)
        .await
        .unwrap();
    assert_eq!(overview.metrics.input_tokens, 150);
    assert_eq!(overview.metrics.prompt_tokens, 1250);
    assert_eq!(overview.metrics.cache_read_tokens, 1000);
    assert_eq!(overview.metrics.token_usage, 1280);
    // Aggregated cache rate is 80%, not the average of model-specific percentages.
    assert_eq!(
        overview.metrics.cache_read_tokens * 100 / overview.metrics.prompt_tokens,
        80
    );
    let calendar = store
        .usage_calendar(start, end, filter, "America/Los_Angeles")
        .await
        .unwrap();
    assert_eq!(calendar.len(), 1);
    assert_eq!(calendar[0].date, "2026-03-08");
    assert_eq!(calendar[0].calls, 2);
    assert_eq!(
        calendar[0].end_ms - calendar[0].usage.bucket_start_ms,
        23 * 3600000
    );
    assert_eq!(
        calendar[0].usage.total_tokens(),
        overview.metrics.token_usage
    );
    let price = ModelTokenPrice {
        input_per_million: 2.0,
        output_per_million: 8.0,
        cache_read_per_million: 0.2,
        cache_write_per_million: 2.5,
    };
    store
        .set_model_pricing(ModelPricingSettings {
            currency: "CNY".into(),
            models: [("a".into(), price.clone())].into(),
        })
        .await
        .unwrap();
    let partial = store
        .cost_estimate(Some(start), Some(end), filter)
        .await
        .unwrap();
    assert_eq!(partial.covered_tokens, 1010);
    assert_eq!(partial.unpriced_tokens, 270);
    assert!(partial
        .models
        .iter()
        .find(|model| model.model_hash.as_deref() == Some("b"))
        .unwrap()
        .amount
        .is_none());
    let mut settings = store.model_pricing().await.unwrap();
    settings.models.insert("b".into(), price);
    store.set_model_pricing(settings).await.unwrap();
    let estimate = store
        .cost_estimate(Some(start), Some(end), filter)
        .await
        .unwrap();
    assert!((estimate.amount - 0.00099).abs() < 1e-12);
    assert_eq!(estimate.covered_tokens, 1280);
    assert_eq!(estimate.unpriced_tokens, 0);
    assert_eq!(estimate.currency, "CNY");
    assert_eq!(estimate.unknown_usage_calls, 0);
    let reopened = Store::connect(&path).await.unwrap();
    assert_eq!(reopened.model_pricing().await.unwrap().models.len(), 2);
    assert_eq!(reopened.model_pricing().await.unwrap().currency, "CNY");
    sqlx::query("UPDATE llm_calls SET input_tokens = NULL, output_tokens = NULL WHERE call_id = 'anthropic'").execute(store.pool()).await.unwrap();
    assert_eq!(
        store
            .cost_estimate(Some(start), Some(end), filter)
            .await
            .unwrap()
            .unknown_usage_calls,
        1
    );
}

#[tokio::test]
async fn empty_year_has_all_days_zero_cost_and_model_filter_is_exact() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::connect(&format!(
        "sqlite://{}",
        temp.path().join("empty.db").display()
    ))
    .await
    .unwrap();
    let start = ms("2025-01-01T00:00:00Z");
    let end = ms("2026-01-01T00:00:00Z");
    let overview = store
        .overview(Some(start), Some(end), None, None)
        .await
        .unwrap();
    assert_eq!(overview.token_usage_series.len(), 365);
    assert_eq!(overview.metrics.token_usage, 0);
    let calendar = store.usage_calendar(start, end, None, "UTC").await.unwrap();
    assert_eq!(calendar.len(), 365);
    assert!(calendar
        .iter()
        .all(|day| day.calls == 0 && day.usage.total_tokens() == 0));
    let estimate = store
        .cost_estimate(Some(start), Some(end), None)
        .await
        .unwrap();
    assert_eq!(estimate.amount, 0.0);
    assert!(estimate.models.is_empty());
    call(&store, "kept", "a", "openai", start, [10, 5, 0, 0]).await;
    call(&store, "excluded", "b", "openai", start, [900, 100, 0, 0]).await;
    let overview = store
        .overview(Some(start), Some(end), Some("[\"a\"]"), None)
        .await
        .unwrap();
    assert_eq!(overview.metrics.llm_calls, 1);
    assert_eq!(overview.metrics.token_usage, 15);
    assert!(store
        .overview(Some(start), Some(end), Some("{\"a\":1}"), None)
        .await
        .is_err());
    assert!(store
        .usage_calendar(start, end, None, "not/a/timezone")
        .await
        .is_err());
}
