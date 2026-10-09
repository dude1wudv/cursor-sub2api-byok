//! Verifies registry ownership follows the transport actor rather than output subscriptions.

#[path = "support/fake_provider.rs"]
mod fake_provider;
#[path = "support/fixtures.rs"]
mod fixtures;

use std::{sync::Arc, time::Duration};

use cursor_server::cursor::{
    conversation::TransportCommand,
    prompting::{PromptAssets, PromptCompiler},
    transport::TransportRegistry,
};

async fn registry() -> (tempfile::TempDir, TransportRegistry) {
    let (directory, store) = fixtures::temp_store().await;
    let assets = PromptAssets::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("prompt/cursor")
            .as_path(),
    )
    .unwrap();
    (
        directory,
        TransportRegistry::new(
            store,
            Arc::new(fake_provider::FakeProvider::default()),
            PromptCompiler::new(assets),
        ),
    )
}

#[tokio::test]
async fn actor_exit_removes_the_matching_transport_and_allows_a_new_generation() {
    let (_directory, registry) = registry().await;
    let first = registry.get_or_create("lifecycle-request").await.unwrap();
    assert!(registry.local("lifecycle-request").await.is_some());

    first.command(TransportCommand::Disconnect).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if registry.local("lifecycle-request").await.is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();

    let second = registry.get_or_create("lifecycle-request").await.unwrap();
    assert_eq!(second.request_id(), "lifecycle-request");
    assert!(registry.local("lifecycle-request").await.is_some());
    second.command(TransportCommand::Disconnect).await.unwrap();
}

#[tokio::test]
async fn dropping_an_output_subscription_does_not_remove_the_transport() {
    let (_directory, registry) = registry().await;
    let handle = registry
        .get_or_create("subscription-request")
        .await
        .unwrap();
    let subscription = handle.subscribe();
    drop(subscription);

    tokio::time::sleep(Duration::from_millis(25)).await;
    assert!(registry.local("subscription-request").await.is_some());

    handle.command(TransportCommand::Disconnect).await.unwrap();
}

async fn http_stream(registry: &TransportRegistry, id: &str) -> axum::response::Response {
    use axum::{body::Body, http::Request};
    use prost::Message;
    use tower::ServiceExt;
    let request = cursor_server::cursor::protocol::proto::agent::v1::BidiRequestId {
        request_id: id.into(),
    };
    cursor_server::api::cursor::router(
        registry.clone(),
        cursor_server::network::NetworkClients::new(registry.store().clone()),
    )
    .unwrap()
    .oneshot(
        Request::post("/agent.v1.AgentService/RunSSE")
            .header("content-type", "application/proto")
            .body(Body::from(request.encode_to_vec()))
            .unwrap(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn http_body_heartbeats_during_silence_without_polluting_replay() {
    use cursor_server::cursor::protocol::{connect, events};
    use futures_util::StreamExt;
    let (_directory, registry) = registry().await;
    let handle = registry.get_or_create("silent").await.unwrap();
    let mut body = http_stream(&registry, "silent")
        .await
        .into_body()
        .into_data_stream();
    let heartbeat = connect::encode_message(&events::heartbeat()).unwrap();
    assert_eq!(body.next().await.unwrap().unwrap(), heartbeat);
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(5)).await;
    assert_eq!(body.next().await.unwrap().unwrap(), heartbeat);
    let mut replay = handle.subscribe();
    assert!(
        replay.try_recv().is_err(),
        "keepalives must not enter replay history"
    );
    registry.shutdown().await;
}

#[tokio::test]
async fn old_http_body_drop_cannot_cancel_a_new_subscriber() {
    use futures_util::StreamExt;
    let (_directory, registry) = registry().await;
    let handle = registry.get_or_create("overlap").await.unwrap();
    let first = http_stream(&registry, "overlap").await;
    let mut second = http_stream(&registry, "overlap")
        .await
        .into_body()
        .into_data_stream();
    drop(first); // Also exercise a response body that was never polled.
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(31)).await;
    tokio::task::yield_now().await;
    assert!(registry.local("overlap").await.is_some());
    let frame = bytes::Bytes::from_static(b"live output");
    assert!(handle.emit_frame(frame.clone()));
    assert_eq!(second.next().await.unwrap().unwrap(), frame);
    registry.shutdown().await;
}

#[tokio::test]
async fn reconnect_invalidates_old_grace_timer_and_last_drop_expires() {
    let (_directory, registry) = registry().await;
    registry.get_or_create("reconnect").await.unwrap();
    let first = http_stream(&registry, "reconnect").await;
    tokio::time::pause();
    drop(first);
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(20)).await;
    let second = http_stream(&registry, "reconnect").await;
    assert_eq!(second.status(), 200);
    drop(second);
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(11)).await;
    tokio::task::yield_now().await;
    assert!(
        registry.local("reconnect").await.is_some(),
        "old timer canceled new grace period"
    );
    tokio::time::advance(Duration::from_secs(20)).await;
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    assert!(registry.local("reconnect").await.is_none());
    let response = http_stream(&registry, "reconnect").await;
    assert_eq!(
        response.status(),
        404,
        "finished requests must return promptly"
    );
}

#[tokio::test]
async fn unknown_route_wait_is_bounded_and_append_can_arrive_later() {
    let (_directory, registry) = registry().await;
    let waiter = registry.clone();
    let task = tokio::spawn(async move { http_stream(&waiter, "late").await });
    tokio::task::yield_now().await;
    registry.get_or_create("late").await.unwrap();
    assert_eq!(task.await.unwrap().status(), 200);
    tokio::time::pause();
    let response = http_stream(&registry, "missing").await;
    assert_eq!(response.status(), 404);
    registry.shutdown().await;
}
