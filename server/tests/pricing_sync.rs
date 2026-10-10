//! All files and stores are synthetic and isolated; no real CC Switch file is accessed.
use cursor_server::{
    model::ModelConfigInput,
    store::{ModelPricingSettings, PricingSyncSettings, Store},
};
use serde_json::{json, Value};

async fn setup() -> (tempfile::TempDir, Store, String) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::connect(&format!(
        "sqlite://{}",
        temp.path().join("test.db").display()
    ))
    .await
    .unwrap();
    let path = temp
        .path()
        .join("model-pricing.json")
        .to_string_lossy()
        .into_owned();
    store
        .set_pricing_sync(PricingSyncSettings {
            enabled: true,
            source_path: path.clone(),
        })
        .await
        .unwrap();
    (temp, store, path)
}

async fn model(store: &Store, id: &str) -> String {
    let input: ModelConfigInput = serde_json::from_value(json!({
        "type":"openai", "display_name": id, "model_id": id,
        "base_url":"https://example.invalid/v1", "api_key":"synthetic", "tooltip_data":"test"
    }))
    .unwrap();
    store.create_model(&input).await.unwrap().model_hash
}

fn price(id: &str, input: &str) -> Value {
    json!({"modelId":id,"displayName":id,"inputCostPerMillion":input,"outputCostPerMillion":"8","cacheReadCostPerMillion":"0.2","cacheCreationCostPerMillion":"2.5"})
}

fn write(path: &str, models: Vec<Value>) {
    std::fs::write(
        path,
        serde_json::to_vec(
            &json!({"version":1,"models":models,"deletedModelIds":[],"modelsDevSync":{}}),
        )
        .unwrap(),
    )
    .unwrap();
}

#[tokio::test]
async fn exact_and_unique_suffix_prices_refresh_without_guessing_model_families() {
    let (_temp, store, path) = setup().await;
    let exact = model(&store, "z-ai/glm-5.3-flash").await;
    let suffix = model(&store, "provider/claude-sonnet-5-5").await;
    let missing = model(&store, "deepseek-v4.1-flash-fast").await;
    let ambiguous = model(&store, "other/ambiguous").await;
    write(
        &path,
        vec![
            price("z-ai/glm-5.3-flash", "2"),
            price("glm-5.3-flash", "99"),
            price("claude-sonnet-5-5", "3"),
            price("deepseek-v4.1-flash", "1"),
            price("a/ambiguous", "1"),
            price("b/ambiguous", "2"),
        ],
    );
    let before = std::fs::read(&path).unwrap();
    let state = store.refresh_pricing_sync().await.unwrap();
    assert_eq!(state.matched.len(), 2);
    assert_eq!(state.unmatched, ["deepseek-v4.1-flash-fast"]);
    assert_eq!(state.ambiguous, ["other/ambiguous"]);
    assert!(state.stale);
    let settings = store.model_pricing().await.unwrap();
    assert_eq!(settings.currency, "USD");
    assert_eq!(settings.models[&exact].input_per_million, 2.0);
    assert_eq!(settings.models[&suffix].input_per_million, 3.0);
    assert_eq!(settings.models[&exact].cache_write_per_million, 2.5);
    assert!(!settings.models.contains_key(&missing));
    assert!(!settings.models.contains_key(&ambiguous));
    assert_eq!(before, std::fs::read(&path).unwrap());
    write(&path, vec![price("z-ai/glm-5.3-flash", "4")]);
    store.refresh_pricing_sync().await.unwrap();
    assert_eq!(
        store.model_pricing().await.unwrap().models[&exact].input_per_million,
        4.0
    );
    // A missing match keeps prior valid data, and is visibly stale.
    assert_eq!(
        store.model_pricing().await.unwrap().models[&suffix].input_per_million,
        3.0
    );
}

#[tokio::test]
async fn partial_missing_invalid_and_deleted_prices_preserve_last_valid_data() {
    let (_temp, store, path) = setup().await;
    let hash = model(&store, "glm-test").await;
    write(&path, vec![price("glm-test", "2")]);
    let good = store.refresh_pricing_sync().await.unwrap();
    for content in [
        "{",
        r#"{"version":2,"models":[]}"#,
        r#"{"version":1,"models":[{"modelId":"glm-test"}]}"#,
    ] {
        std::fs::write(&path, content).unwrap();
        let state = store.refresh_pricing_sync().await.unwrap();
        assert!(state.stale && state.error.is_some());
        assert_eq!(state.synced_at_ms, good.synced_at_ms);
        assert_eq!(
            store.model_pricing().await.unwrap().models[&hash].input_per_million,
            2.0
        );
    }
    for bad in ["NaN", "-1", "inf", "1000000001", ""] {
        write(&path, vec![price("glm-test", bad)]);
        assert!(store.refresh_pricing_sync().await.unwrap().error.is_some());
    }
    write(&path, vec![price("glm-test", "1"), price("glm-test", "2")]);
    assert!(store.refresh_pricing_sync().await.unwrap().error.is_some());
    std::fs::write(
        &path,
        serde_json::to_vec(
            &json!({"version":1,"models":[price("glm-test","5")],"deletedModelIds":["glm-test"]}),
        )
        .unwrap(),
    )
    .unwrap();
    let state = store.refresh_pricing_sync().await.unwrap();
    assert!(state.stale);
    assert_eq!(state.unmatched, ["glm-test"]);
    std::fs::remove_file(&path).unwrap();
    assert!(store.refresh_pricing_sync().await.unwrap().error.is_some());
    assert_eq!(
        store.model_pricing().await.unwrap().models[&hash].input_per_million,
        2.0
    );
}

#[tokio::test]
async fn disable_unlocks_manual_prices_and_concurrent_refresh_cannot_reenable_it() {
    let (_temp, store, path) = setup().await;
    model(&store, "test-model").await;
    write(&path, vec![price("test-model", "2")]);
    assert!(store
        .set_manual_model_pricing(ModelPricingSettings::default())
        .await
        .is_err());
    let (refresh, disabled) = tokio::join!(
        store.refresh_pricing_sync(),
        store.set_pricing_sync(PricingSyncSettings {
            enabled: false,
            source_path: path.clone()
        })
    );
    refresh.unwrap();
    disabled.unwrap();
    let manual = ModelPricingSettings {
        currency: "CNY".into(),
        ..Default::default()
    };
    store.set_manual_model_pricing(manual).await.unwrap();
    store.refresh_pricing_sync().await.unwrap();
    assert!(!store.pricing_sync_status().await.unwrap().settings.enabled);
    assert_eq!(store.model_pricing().await.unwrap().currency, "CNY");
    store
        .set_pricing_sync(PricingSyncSettings {
            enabled: true,
            source_path: path,
        })
        .await
        .unwrap();
    assert!(store
        .refresh_pricing_sync()
        .await
        .unwrap()
        .error
        .unwrap()
        .contains("USD"));
    assert_eq!(store.model_pricing().await.unwrap().currency, "CNY");
}
