//! Real loopback HTTP controller with temporary data and no trust-store changes.
use cursor_server::{config::RuntimePaths, App, Config};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
#[tokio::test]
async fn clean_start_control_auth_and_connection_contract() {
    let dir = tempfile::Builder::new()
        .prefix("Sub2API isolated space ")
        .tempdir()
        .unwrap();
    let paths = RuntimePaths::resolve(
        Some(dir.path().join("controller")),
        Some(dir.path().join("Cursor Profile")),
    )
    .unwrap();
    std::fs::create_dir_all(paths.cursor_settings.parent().unwrap()).unwrap();
    let sentinel = paths.cursor_settings.parent().unwrap().join("state.vscdb");
    std::fs::write(&sentinel, b"sentinel-never-opened").unwrap();
    let app = App::new(Config::desktop_with_paths(paths.clone()).unwrap())
        .await
        .unwrap();
    let store = app.store();
    let token = app.control_token().to_owned();
    let listener = app.bind().await.unwrap();
    let address = listener.local_addr().unwrap();
    let shutdown = CancellationToken::new();
    let task = tokio::spawn(app.serve_on(listener, shutdown.clone()));
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let root = format!("http://{address}/__byok-api__/api");
    let status_url = format!("{root}/harness/cursor/status");
    for (token_value, origin, host) in [
        ("", None, None),
        ("wrong", None, None),
        (token.as_str(), Some("https://evil.example"), None),
        (token.as_str(), None, Some("localhost:123")),
    ] {
        let mut request = client
            .get(&status_url)
            .header("X-Sub2API-Control-Token", token_value);
        if let Some(origin) = origin {
            request = request.header("Origin", origin);
        }
        if let Some(host) = host {
            request = request.header("Host", host);
        }
        assert_eq!(request.send().await.unwrap().status(), 403);
    }
    let status: Value = client
        .get(&status_url)
        .header("X-Sub2API-Control-Token", &token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["integration"], "disabled");
    assert_eq!(status["ca"], "missing");
    assert!(!paths.cursor_settings.exists());
    assert!(!paths.data_dir.join("ca").exists());
    assert!(!paths.data_dir.join("plugins").exists());
    for route in [
        "promotions",
        "plugins",
        "settings/external-api",
        "settings/tab",
        "settings/commit",
    ] {
        assert_eq!(
            client
                .get(format!("{root}/{route}"))
                .header("X-Sub2API-Control-Token", &token)
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
    }
    let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let fixture_base = format!("http://{}/v1", upstream.local_addr().unwrap());
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let capture = seen.clone();
    let fixture = axum::Router::new().fallback(axum::routing::post(move |headers:axum::http::HeaderMap, uri:axum::http::Uri| {
        let capture = capture.clone();
        async move {
            assert!(!headers.contains_key("x-sub2api-control-token"));
            let path=uri.path().to_string();
            if path == "/v1/responses" { assert_eq!(headers["authorization"],"Bearer synthetic-secret"); }
            else { assert_eq!(path,"/v1/messages"); assert_eq!(headers["x-api-key"],"synthetic-secret"); }
            capture.lock().unwrap().push(path.clone());
            let events = if path == "/v1/responses" {
                vec![json!({"type":"response.output_text.delta","delta":"fixture ok"}),json!({"type":"response.completed","response":{"status":"completed"}})]
            } else {
                vec![json!({"type":"message_start","message":{"id":"fixture","usage":{"input_tokens":1,"output_tokens":0}}}),
                    json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"fixture ok"}}),
                    json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":2}}),json!({"type":"message_stop"})]
            };
            ([("content-type","text/event-stream")],events.iter().map(|e|format!("event: {}\ndata: {e}\n\n",e["type"].as_str().unwrap())).collect::<String>())
        }
    }));
    let upstream_task = tokio::spawn(async move {
        axum::serve(upstream, fixture).await.unwrap();
    });
    let response = client
        .put(format!("{root}/sub2api/connection"))
        .header("X-Sub2API-Control-Token", &token)
        .json(&json!({"base_url":fixture_base,"api_key":"synthetic-secret"}))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body = response.text().await.unwrap();
    assert!(!body.contains("synthetic-secret"));
    let response=client.post(format!("{root}/models")).header("X-Sub2API-Control-Token",&token).json(&json!({"models":[{"display_name":"m","model_id":"m","type":"openai","api_key":"forged"}]})).send().await.unwrap();
    assert_eq!(response.status(), 400);
    for (model, kind) in [("gpt-fixture", "openai"), ("claude-fixture", "anthropic")] {
        let created: Value = client
            .post(format!("{root}/models"))
            .header("X-Sub2API-Control-Token", &token)
            .json(&json!({"models":[{"display_name":model,"model_id":model,"type":kind}]}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(!created.to_string().contains("synthetic-secret"));
        let hash = created[0]["model_hash"].as_str().unwrap();
        let response = client
            .post(format!("{root}/models/{hash}/test/fixture"))
            .header("X-Sub2API-Control-Token", &token)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let body = response.text().await.unwrap();
        assert!(status.is_success(), "{status} {body}");
        assert!(body.contains("fixture ok"));
    }
    assert_eq!(*seen.lock().unwrap(), vec!["/v1/responses", "/v1/messages"]);
    upstream_task.abort();
    for _ in 0..2 {
        let response = client
            .put(format!("{root}/harness/cursor/enabled"))
            .header("X-Sub2API-Control-Token", &token)
            .json(&json!({"enabled":false}))
            .send()
            .await
            .unwrap();
        let status = response.status();
        let body = response.text().await.unwrap();
        assert!(status.is_success(), "{status} {body}");
    }
    assert_eq!(std::fs::read(sentinel).unwrap(), b"sentinel-never-opened");
    shutdown.cancel();
    task.await.unwrap().unwrap();
    store.pool().close().await;
}
#[tokio::test]
async fn offline_restore_with_no_journal_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::resolve(
        Some(dir.path().join("data")),
        Some(dir.path().join("cursor")),
    )
    .unwrap();
    cursor_server::local_app::restore_paths(&paths)
        .await
        .unwrap();
    assert!(!paths.data_dir.exists());
    assert!(!paths.cursor_settings.exists());
}

#[test]
fn existing_settings_resolves_to_a_readable_file_without_trailing_separator() {
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join("Cursor Profile");
    std::fs::create_dir_all(profile.join("User")).unwrap();
    std::fs::write(profile.join("User/settings.json"), b"{}").unwrap();
    let paths = RuntimePaths::resolve(Some(dir.path().join("data")), Some(profile)).unwrap();
    assert_eq!(std::fs::read(paths.cursor_settings).unwrap(), b"{}");
    assert!(RuntimePaths::resolve(
        Some(dir.path().join("Shared")),
        Some(dir.path().join("shared"))
    )
    .is_err());
}
