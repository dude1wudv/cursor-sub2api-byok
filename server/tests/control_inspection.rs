//! Control API inspection and network settings over isolated local state.
use cursor_server::{config::RuntimePaths, network::NetworkClients, store::Store, App, Config};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

async fn start_app(
    dir: &tempfile::TempDir,
) -> (
    String,
    String,
    Store,
    CancellationToken,
    tokio::task::JoinHandle<cursor_server::Result<()>>,
) {
    let paths = RuntimePaths::resolve(
        Some(dir.path().join("data")),
        Some(dir.path().join("Cursor Profile")),
    )
    .unwrap();
    let app = App::new(Config::desktop_with_paths(paths).unwrap())
        .await
        .unwrap();
    let store = app.store();
    let token = app.control_token().to_owned();
    let listener = app.bind().await.unwrap();
    let address = listener.local_addr().unwrap();
    let shutdown = CancellationToken::new();
    let task = tokio::spawn(app.serve_on(listener, shutdown.clone()));
    (
        format!("http://{address}/__byok-api__/api"),
        token,
        store,
        shutdown,
        task,
    )
}

async fn insert_call(
    store: &Store,
    id: &str,
    model: &str,
    conversation: &str,
    status: &str,
    created_at_ms: i64,
) {
    sqlx::query("INSERT INTO llm_calls(call_id,run_id,conversation_id,provider_call_index,model_hash,provider_type,provider_url,request_type,request_url,model_id,display_name,status,created_at_ms,input_tokens,output_tokens,message_count,tool_count,detailed) VALUES(?, 'synthetic-run', ?, 0, ?, 'openai', 'https://provider.example', 'responses', 'https://provider.example/v1/responses', ?, ?, ?, ?, 10, 5, 1, 0, 0)")
        .bind(id).bind(conversation).bind(model).bind(model).bind(model).bind(status).bind(created_at_ms)
        .execute(store.pool()).await.unwrap();
}

#[tokio::test]
async fn call_filters_pagination_and_parent_diagnostics_are_exact() {
    let dir = tempfile::tempdir().unwrap();
    let (root, token, store, shutdown, task) = start_app(&dir).await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    insert_call(
        &store,
        "outside",
        "alpha",
        "conversation-a",
        "completed",
        9_999,
    )
    .await;
    insert_call(
        &store,
        "alpha-start",
        "alpha",
        "conversation-a",
        "completed",
        10_000,
    )
    .await;
    insert_call(
        &store,
        "beta-failed",
        "beta",
        "conversation-b",
        "failed",
        20_000,
    )
    .await;
    insert_call(
        &store,
        "alpha-error",
        "alpha",
        "conversation-a",
        "error",
        30_000,
    )
    .await;
    insert_call(
        &store,
        "alpha-end",
        "alpha",
        "conversation-a",
        "cancelled",
        40_000,
    )
    .await;

    let get_calls = |query: &str| {
        client
            .get(format!("{root}/llm-calls?{query}"))
            .header("X-Sub2API-Control-Token", &token)
    };
    let page_one: Value = get_calls("page=1&limit=2")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(page_one["total"], 5);
    assert_eq!(page_one["items"][0]["call_id"], "alpha-end");
    assert_eq!(page_one["items"][1]["call_id"], "alpha-error");
    let page_two: Value = get_calls("page=2&limit=2")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(page_two["items"][0]["call_id"], "beta-failed");
    assert_eq!(page_two["items"][1]["call_id"], "alpha-start");

    let bounded: Value =
        get_calls("start_ms=10000&end_ms=40000&model_hash=alpha&conversation_id=conversation-a")
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
    assert_eq!(bounded["total"], 2);
    assert_eq!(bounded["items"][1]["call_id"], "alpha-start");
    let failed: Value = get_calls("status=failed")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let failed_statuses: Vec<&str> = failed["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["status"].as_str().unwrap())
        .collect();
    assert_eq!(failed["total"], 3);
    assert!(failed_statuses.contains(&"failed"));
    assert!(failed_statuses.contains(&"error"));
    assert!(failed_statuses.contains(&"cancelled"));
    let invalid = get_calls("status=error").send().await.unwrap();
    assert_eq!(invalid.status(), reqwest::StatusCode::BAD_REQUEST);

    for (request_id, parent_id) in [
        ("parent-req", None),
        ("child-req", Some("parent-req")),
        ("unrelated", None),
    ] {
        store
            .record_route_diagnostic(&cursor_server::store::RouteDiagnostic {
                created_at_ms: 1,
                request_id: Some(request_id.into()),
                parent_request_id: parent_id.map(str::to_owned),
                conversation_id: Some("synthetic-conversation".into()),
                parent_tool_call_id: Some("synthetic-task".into()),
                method: "POST".into(),
                path: "/v1/responses".into(),
                route: "sub2api".into(),
                stage: "upstream_response".into(),
                http_status: 502,
                duration_ms: 25,
                ..Default::default()
            })
            .await
            .unwrap();
    }
    let related: Value = client
        .get(format!("{root}/route-diagnostics?request_id=parent-req"))
        .header("X-Sub2API-Control-Token", &token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(related.as_array().unwrap().len(), 2);
    assert!(related
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["request_id"] == "child-req" && row["parent_request_id"] == "parent-req"));
    assert!(related
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["request_id"] != "unrelated"));

    shutdown.cancel();
    task.await.unwrap().unwrap();
    store.pool().close().await;
}

#[tokio::test]
async fn port_zero_occupied_port_and_encrypted_proxy_password_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let (root, token, store, shutdown, task) = start_app(&dir).await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let ports_url = format!("{root}/network/ports");
    let occupied = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let occupied_port = occupied.local_addr().unwrap().port();
    let rejected = client
        .put(&ports_url)
        .header("X-Sub2API-Control-Token", &token)
        .json(&json!({"proxy_port":occupied_port,"service_port":0}))
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), reqwest::StatusCode::BAD_REQUEST);
    assert_eq!(store.port_settings().await.unwrap(), Default::default());
    drop(occupied);

    let saved_ports: Value = client
        .put(&ports_url)
        .header("X-Sub2API-Control-Token", &token)
        .json(&json!({"proxy_port":0,"service_port":0}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        saved_ports["ports"],
        json!({"proxy_port":0,"service_port":0})
    );
    assert_eq!(store.port_settings().await.unwrap(), Default::default());

    let proxy_url = format!("{root}/network/proxy");
    let saved: Value = client.put(&proxy_url).header("X-Sub2API-Control-Token", &token)
        .json(&json!({"mode":"custom","address":"http://127.0.0.1:7890","auth_enabled":true,"username":"synthetic-user","password":"synthetic-proxy-password"}))
        .send().await.unwrap().error_for_status().unwrap().json().await.unwrap();
    assert_eq!(saved["outbound"]["has_password"], true);
    assert!(saved.to_string().find("synthetic-proxy-password").is_none());
    let stored: String = sqlx::query_scalar(
        "SELECT value_json FROM service_settings WHERE setting_key='outbound_proxy'",
    )
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert!(!stored.contains("synthetic-proxy-password"));
    assert!(stored.contains("dpapi:v1:"));

    let db_url = format!(
        "sqlite://{}",
        dir.path().join("data/cursor-sub2api.db").display()
    );
    let reopened = Store::connect(&db_url).await.unwrap();
    assert!(reopened.proxy_settings().await.unwrap().has_password);
    // Building the configured client decrypts the persisted CurrentUser DPAPI value.
    let _client = NetworkClients::new(reopened.clone())
        .default_client()
        .await
        .unwrap();
    let preserved: Value = client.put(&proxy_url).header("X-Sub2API-Control-Token", &token)
        .json(&json!({"mode":"custom","address":"http://127.0.0.1:7890","auth_enabled":true,"username":"synthetic-user"}))
        .send().await.unwrap().error_for_status().unwrap().json().await.unwrap();
    assert_eq!(preserved["outbound"]["has_password"], true);
    let _client_after_preserve = NetworkClients::new(reopened.clone())
        .default_client()
        .await
        .unwrap();

    shutdown.cancel();
    task.await.unwrap().unwrap();
    reopened.pool().close().await;
    store.pool().close().await;
}
