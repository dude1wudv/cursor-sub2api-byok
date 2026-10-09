//! Synthetic, isolated DPAPI/SQLite connection and rollback acceptance tests.
use cursor_server::{
    model::model_hash,
    store::{Store, Sub2ApiConnectionInput, Sub2ApiModelInput},
};
use serde_json::json;
fn input(id: &str, kind: &str) -> Sub2ApiModelInput {
    serde_json::from_value(json!({"display_name":id,"model_id":id,"type":kind,"sort_order":if kind == "openai" {0} else {1}})).unwrap()
}
#[tokio::test]
async fn connection_is_shared_encrypted_and_syncs_models_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("connection.db");
    let store = Store::connect(&format!("sqlite://{}", db.display()))
        .await
        .unwrap();
    assert!(!store.sub2api_connection().await.unwrap().has_api_key);
    let key = "synthetic-sub2api-private-key"; // gitleaks:allow -- synthetic fixture only
    let connection = store
        .set_sub2api_connection(Sub2ApiConnectionInput {
            base_url: "https://example.com/v1".into(),
            api_key: Some(key.into()),
        })
        .await
        .unwrap();
    assert!(connection.has_api_key);
    for (id, kind, path) in [
        ("gpt-fixture", "openai", "/v1/responses"),
        ("claude-fixture", "anthropic", "/v1/messages"),
    ] {
        let configured = store.sub2api_model_input(&input(id, kind)).await.unwrap();
        let m = store.create_model(&configured).await.unwrap();
        assert_eq!(m.api_key, key);
        assert_eq!(
            m.request_url().unwrap(),
            format!("https://example.com{path}")
        );
        assert_eq!(m.model_hash, model_hash(&configured).unwrap());
        assert!(m.reasoning_effort.is_none() && m.anthropic_thinking_effort.is_none());
    }
    let before = store.models().await.unwrap();
    let saved_before: String = sqlx::query_scalar(
        "SELECT value_json FROM service_settings WHERE setting_key='sub2api_connection'",
    )
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert!(!saved_before.contains(key));
    assert!(!serde_json::to_string(&before).unwrap().contains(key));
    assert!(!format!("{before:?}").contains(key));
    // Abort the second insertion after the first model has already changed in the transaction.
    sqlx::query("CREATE TRIGGER synthetic_failure BEFORE INSERT ON model_configs WHEN NEW.model_id='claude-fixture' BEGIN SELECT RAISE(ABORT,'synthetic mid-transaction failure'); END").execute(store.pool()).await.unwrap();
    assert!(store
        .set_sub2api_connection(Sub2ApiConnectionInput {
            base_url: "https://other.example".into(),
            api_key: Some("synthetic-replacement-key".into())
        })
        .await
        .is_err());
    assert_eq!(
        store
            .models()
            .await
            .unwrap()
            .iter()
            .map(|m| &m.model_hash)
            .collect::<Vec<_>>(),
        before.iter().map(|m| &m.model_hash).collect::<Vec<_>>()
    );
    let saved_after: String = sqlx::query_scalar(
        "SELECT value_json FROM service_settings WHERE setting_key='sub2api_connection'",
    )
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert_eq!(saved_before, saved_after);
    sqlx::query("DROP TRIGGER synthetic_failure")
        .execute(store.pool())
        .await
        .unwrap();
    store
        .set_sub2api_connection(Sub2ApiConnectionInput {
            base_url: "https://other.example".into(),
            api_key: None,
        })
        .await
        .unwrap();
    for m in store.models().await.unwrap() {
        assert_eq!(m.api_key, key);
        assert_eq!(m.base_url, "https://other.example");
    }
    // WAL is included in the check; the write path never binds a plaintext key.
    for file in [&db, &db.with_extension("db-wal")] {
        if let Ok(bytes) = std::fs::read(file) {
            assert!(!bytes.windows(key.len()).any(|w| w == key.as_bytes()));
        }
    }
    store.pool().close().await;
}
#[tokio::test]
async fn rejects_unsafe_bases_empty_keys_and_forged_model_fields() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::connect(&format!(
        "sqlite://{}",
        dir.path().join("test.db").display()
    ))
    .await
    .unwrap();
    for base in [
        "http://example.com",
        "https://u:p@example.com",
        "https://example.com?key=x",
        "https://example.com/#x",
        "https://example.com/v1/responses",
        "file:///x",
    ] {
        assert!(store
            .set_sub2api_connection(Sub2ApiConnectionInput {
                base_url: base.into(),
                api_key: Some("synthetic".into())
            })
            .await
            .is_err());
    }
    assert!(store
        .set_sub2api_connection(Sub2ApiConnectionInput {
            base_url: "https://example.com".into(),
            api_key: Some(" ".into())
        })
        .await
        .is_err());
    for field in [
        "api_key",
        "base_url",
        "custom_headers",
        "openai_extra_params",
        "prompt",
    ] {
        let mut value = json!({"display_name":"m","model_id":"m","type":"openai"});
        value[field] = json!("forged");
        assert!(serde_json::from_value::<Sub2ApiModelInput>(value).is_err());
    }
    store.pool().close().await;
}

#[tokio::test]
async fn allowed_efforts_roundtrip_validation_and_connection_sync() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::connect(&format!("sqlite://{}", dir.path().join("efforts.db").display())).await.unwrap();
    store.set_sub2api_connection(Sub2ApiConnectionInput { base_url: "https://example.com".into(), api_key: Some("synthetic".into()) }).await.unwrap();
    for kind in ["openai", "anthropic"] {
        let mut input = input(kind, kind);
        input.allowed_reasoning_efforts = vec!["low".into(), "high".into(), "high".into()];
        input.reasoning_effort = Some("high".into());
        let configured = store.sub2api_model_input(&input).await.unwrap();
        let saved = store.create_model(&configured).await.unwrap();
        assert_eq!(saved.allowed_reasoning_efforts, vec!["low", "high"]);
        input.reasoning_effort = Some("max".into());
        assert!(store.sub2api_model_input(&input).await.is_err());
        input.reasoning_effort = None;
        input.allowed_reasoning_efforts = vec!["invalid".into()];
        assert!(store.sub2api_model_input(&input).await.is_err());
        input.allowed_reasoning_efforts = vec!["medium".into()];
        input.reasoning_effort = Some("medium".into());
        store.update_model(&saved.model_hash, &store.sub2api_model_input(&input).await.unwrap()).await.unwrap();
    }
    store.set_sub2api_connection(Sub2ApiConnectionInput { base_url: "https://other.example/v1".into(), api_key: None }).await.unwrap();
    for saved in store.models().await.unwrap() {
        assert_eq!(saved.allowed_reasoning_efforts, vec!["medium"]);
        assert_eq!(saved.reasoning_effort.or(saved.anthropic_thinking_effort).as_deref(), Some("medium"));
    }
    store.pool().close().await;
}
