//! Model management uses synthetic SQLite and a deliberately non-cooperative stream.
use cursor_server::{
    config::RuntimePaths,
    control::ControlService,
    model::ModelInvocation,
    provider::{Provider, ProviderStream},
    store::{Store, Sub2ApiConnectionInput, Sub2ApiModelInput},
    Error,
};
use serde_json::json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio_util::sync::CancellationToken;

struct PendingProvider {
    started: Arc<tokio::sync::Notify>,
    dropped: Arc<AtomicUsize>,
}
struct Dropped(Arc<AtomicUsize>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
impl Provider for PendingProvider {
    fn stream(&self, invocation: ModelInvocation, _: CancellationToken) -> ProviderStream {
        assert_eq!(invocation.request.model.max_output_tokens, Some(256));
        let started = self.started.clone();
        let dropped = self.dropped.clone();
        Box::pin(async_stream::try_stream! {
            let _guard = Dropped(dropped);
            started.notify_one();
            std::future::pending::<()>().await;
            yield cursor_server::provider::ModelEvent::TextDelta("unreachable".into());
        })
    }
}

async fn setup() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::connect(&format!(
        "sqlite://{}",
        dir.path().join("models.db").display()
    ))
    .await
    .unwrap();
    store
        .set_sub2api_connection(Sub2ApiConnectionInput {
            base_url: "https://synthetic.example/v1".into(),
            api_key: Some("synthetic-model-management-key".into()), // gitleaks:allow
        })
        .await
        .unwrap();
    (dir, store)
}

#[tokio::test]
async fn copy_group_and_order_survive_shared_connection_sync() {
    let (_dir, store) = setup().await;
    let input: Sub2ApiModelInput = serde_json::from_value(json!({
        "display_name":"Fixture", "model_id":"fixture", "type":"openai", "group_name":"开发", "sort_order":3
    })).unwrap();
    let original = store
        .create_model(&store.sub2api_model_input(&input).await.unwrap())
        .await
        .unwrap();
    let mut duplicate = input.clone();
    duplicate.display_name = "Fixture 副本".into();
    duplicate.sort_order = 4;
    let copy = store
        .create_model(&store.sub2api_model_input(&duplicate).await.unwrap())
        .await
        .unwrap();
    assert_ne!(original.model_hash, copy.model_hash);
    assert_eq!(original.api_key, copy.api_key);
    let stored_keys: Vec<String> = sqlx::query_scalar("SELECT api_key FROM model_configs")
        .fetch_all(store.pool())
        .await
        .unwrap();
    assert_eq!(
        stored_keys,
        ["sub2api-connection:v1", "sub2api-connection:v1"]
    );
    let shared: String = sqlx::query_scalar("SELECT json_extract(value_json, '$.api_key_protected') FROM service_settings WHERE setting_key = 'sub2api_connection'")
        .fetch_one(store.pool()).await.unwrap();
    assert!(!shared.contains("synthetic-model-management-key"));
    assert!(shared.starts_with("dpapi:v1:"));
    assert_eq!(original.api_key, "synthetic-model-management-key");
    store
        .reorder_models(&[copy.model_hash.clone(), original.model_hash.clone()])
        .await
        .unwrap();
    store
        .set_sub2api_connection(Sub2ApiConnectionInput {
            base_url: "https://replacement.example/v1".into(),
            api_key: None,
        })
        .await
        .unwrap();
    let models = store.models().await.unwrap();
    assert_eq!(
        models
            .iter()
            .map(|model| model.display_name.as_str())
            .collect::<Vec<_>>(),
        ["Fixture 副本", "Fixture"]
    );
    assert!(models
        .iter()
        .all(|model| model.group_name.as_deref() == Some("开发")));
    assert!(models
        .iter()
        .all(|model| model.base_url == "https://replacement.example/v1"));
    let references: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM model_configs WHERE api_key = 'sub2api-connection:v1'",
    )
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert_eq!(references, 2);
    let public = serde_json::to_string(&models).unwrap();
    assert!(!public.contains("synthetic-model-management-key"));
    assert!(serde_json::from_value::<Sub2ApiModelInput>(json!({
        "display_name":"Reject", "model_id":"fixture", "type":"openai", "api_key":"forbidden-copy-key"
    })).is_err());
}

#[tokio::test]
async fn no_op_connection_save_replaces_historical_model_ciphertext_with_reference() {
    let (_dir, store) = setup().await;
    let input: Sub2ApiModelInput = serde_json::from_value(
        json!({"display_name":"Fixture", "model_id":"fixture", "type":"openai"}),
    )
    .unwrap();
    let configured = store.sub2api_model_input(&input).await.unwrap();
    let original = store.create_model(&configured).await.unwrap();
    let old_cipher: String = sqlx::query_scalar("SELECT json_extract(value_json, '$.api_key_protected') FROM service_settings WHERE setting_key = 'sub2api_connection'")
        .fetch_one(store.pool()).await.unwrap();
    sqlx::query("UPDATE model_configs SET api_key = ? WHERE model_hash = ?")
        .bind(&old_cipher)
        .bind(&original.model_hash)
        .execute(store.pool())
        .await
        .unwrap();
    store
        .set_sub2api_connection(Sub2ApiConnectionInput {
            base_url: "https://synthetic.example/v1".into(),
            api_key: None,
        })
        .await
        .unwrap();
    let reference: String =
        sqlx::query_scalar("SELECT api_key FROM model_configs WHERE model_hash = ?")
            .bind(&original.model_hash)
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(reference, "sub2api-connection:v1");
    assert_ne!(reference, old_cipher);
    assert_eq!(
        store
            .model(&original.model_hash)
            .await
            .unwrap()
            .unwrap()
            .api_key,
        original.api_key
    );

    // A stale caller must not scatter a second key after the connection changed.
    let mut stale = configured;
    stale.display_name = "Stale copy".into();
    stale.api_key = "synthetic-stale-key".into(); // gitleaks:allow
    assert!(matches!(
        store.create_model(&stale).await,
        Err(Error::Config(_))
    ));
    assert_eq!(store.models().await.unwrap().len(), 1);
}

#[tokio::test]
async fn dangling_shared_reference_is_an_explicit_error_not_an_empty_key() {
    let (_dir, store) = setup().await;
    let input: Sub2ApiModelInput = serde_json::from_value(
        json!({"display_name":"Fixture", "model_id":"fixture", "type":"openai"}),
    )
    .unwrap();
    let original = store
        .create_model(&store.sub2api_model_input(&input).await.unwrap())
        .await
        .unwrap();
    sqlx::query("DELETE FROM service_settings WHERE setting_key = 'sub2api_connection'")
        .execute(store.pool())
        .await
        .unwrap();
    assert!(matches!(
        store.model(&original.model_hash).await,
        Err(Error::Config(_))
    ));
    assert!(matches!(store.models().await, Err(Error::Config(_))));
}

#[tokio::test]
async fn cancellation_drops_unresponsive_upstream_and_pre_cancel_does_not_dispatch() {
    let (dir, store) = setup().await;
    let input: Sub2ApiModelInput = serde_json::from_value(
        json!({"display_name":"Fixture", "model_id":"fixture", "type":"openai"}),
    )
    .unwrap();
    let model = store
        .create_model(&store.sub2api_model_input(&input).await.unwrap())
        .await
        .unwrap();
    let started = Arc::new(tokio::sync::Notify::new());
    let dropped = Arc::new(AtomicUsize::new(0));
    let paths = RuntimePaths::resolve(
        Some(dir.path().join("controller")),
        Some(dir.path().join("cursor")),
    )
    .unwrap();
    let service = ControlService::new(
        store,
        Arc::new(PendingProvider {
            started: started.clone(),
            dropped: dropped.clone(),
        }),
        &paths,
    )
    .unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let run_service = service.clone();
    let run_hash = model.model_hash.clone();
    let run_id = id.clone();
    let task = tokio::spawn(async move { run_service.test_model(&run_hash, &run_id).await });
    tokio::time::timeout(std::time::Duration::from_secs(3), started.notified())
        .await
        .unwrap();
    service.cancel_model_test(&model.model_hash, &id).unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result, Err(Error::Cancelled)));
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert!(matches!(
        service.test_model(&model.model_hash, &id).await,
        Err(Error::ControllerConflict { .. })
    ));
    let next = uuid::Uuid::new_v4().to_string();
    service.cancel_model_test(&model.model_hash, &next).unwrap();
    assert!(matches!(
        service.test_model(&model.model_hash, &next).await,
        Err(Error::Cancelled)
    ));
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
}
