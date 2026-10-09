//! Loopback-only discovery contract; no live key or trust-store changes.
use cursor_server::{config::RuntimePaths, App, Config};
use serde_json::{json, Value};
use std::sync::{atomic::{AtomicU8, Ordering}, Arc};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn discovery_auth_paths_dedup_errors_redirect_and_size_bound() {
    let dir = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::resolve(Some(dir.path().join("data")), Some(dir.path().join("cursor"))).unwrap();
    let app = App::new(Config::desktop_with_paths(paths).unwrap()).await.unwrap();
    let store = app.store();
    let token = app.control_token().to_owned();
    let listener = app.bind().await.unwrap();
    let root = format!("http://{}/__byok-api__/api", listener.local_addr().unwrap());
    let stop = CancellationToken::new();
    let server = tokio::spawn(app.serve_on(listener, stop.clone()));
    let fixture_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let fixture_base = format!("http://{}", fixture_listener.local_addr().unwrap());
    let mode = Arc::new(AtomicU8::new(0));
    let captured_mode = mode.clone();
    let fixture = axum::Router::new().route("/v1/models", axum::routing::get(move |headers: axum::http::HeaderMap| {
        let mode = captured_mode.load(Ordering::SeqCst);
        async move {
            assert_eq!(headers["authorization"], "Bearer synthetic-discovery");
            assert!(!headers.contains_key("x-sub2api-control-token"));
            let (status, body) = match mode {
                1 => (401, "synthetic-discovery must never be reflected".into()),
                2 => (200, "invalid JSON".into()),
                3 => (302, String::new()),
                4 => (200, "x".repeat(2 * 1024 * 1024 + 1)),
                _ => (200, json!({"data":[{"id":"gpt-fixture"},{"id":"claude-fixture"},{"id":"gpt-fixture"},{"id":" "}]}).to_string()),
            };
            axum::http::Response::builder().status(status).header("location", "/must-not-follow").body(body).unwrap()
        }
    })).route("/must-not-follow", axum::routing::get(|| async { panic!("must not follow a redirect with credentials"); #[allow(unreachable_code)] "" }));
    let upstream = tokio::spawn(async move { axum::serve(fixture_listener, fixture).await.unwrap(); });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let url = format!("{root}/sub2api/models");
    assert_eq!(client.get(&url).send().await.unwrap().status(), 403);
    assert!(!client.get(&url).header("X-Sub2API-Control-Token", &token).send().await.unwrap().status().is_success());
    for suffix in ["", "/v1"] {
        client.put(format!("{root}/sub2api/connection")).header("X-Sub2API-Control-Token", &token)
            .json(&json!({"base_url":format!("{fixture_base}{suffix}"),"api_key":"synthetic-discovery"})).send().await.unwrap().error_for_status().unwrap();
        let models: Value = client.get(&url).header("X-Sub2API-Control-Token", &token).send().await.unwrap().error_for_status().unwrap().json().await.unwrap();
        assert_eq!(models, json!([{"id":"claude-fixture"},{"id":"gpt-fixture"}]));
    }
    for error_mode in 1..=4 {
        mode.store(error_mode, Ordering::SeqCst);
        let response = client.get(&url).header("X-Sub2API-Control-Token", &token).send().await.unwrap();
        assert!(!response.status().is_success());
        assert!(!response.text().await.unwrap().contains("synthetic-discovery"));
    }
    // Bulk model validation must finish before any insertion.
    let response = client.post(format!("{root}/models")).header("X-Sub2API-Control-Token", &token).json(&json!({"models":[
        {"display_name":"valid","model_id":"valid","type":"openai"},
        {"display_name":"invalid","model_id":"invalid","type":"openai","allowed_reasoning_efforts":["low"],"reasoning_effort":"max"}
    ]})).send().await.unwrap();
    assert!(!response.status().is_success());
    assert!(store.models().await.unwrap().is_empty());
    upstream.abort(); stop.cancel(); server.await.unwrap().unwrap(); store.pool().close().await;
}
