//! Verifies captured Cursor Connect framing and protobuf compatibility.
#[path = "support/fake_cursor.rs"]
mod fake_cursor;
#[path = "support/fake_provider.rs"]
mod fake_provider;
#[path = "support/fixtures.rs"]
mod fixtures;

use std::{io::Write, sync::Arc};

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
};
use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine};
use cursor_server::{
    api::cursor,
    cursor::prompting::{PromptAssets, PromptCompiler},
    cursor::protocol::{
        connect,
        proto::{agent::v1 as pb, aiserver::v1 as ai},
    },
    cursor::transport::TransportRegistry,
    network::NetworkClients,
};
use flate2::{write::GzEncoder, Compression};
use prost::Message;
use tower::ServiceExt;

#[test]
fn connect_envelope_is_flag_plus_big_endian_length_plus_protobuf() {
    let message = pb::BidiRequestId {
        request_id: "abc".into(),
    };
    let frame = connect::encode_message(&message).unwrap();
    assert_eq!(&frame[..5], &[0, 0, 0, 0, 5]);
    let decoded: pb::BidiRequestId = fake_cursor::decode_single(&frame).unwrap();
    assert_eq!(decoded.request_id, "abc");
}

#[test]
fn end_stream_matches_captured_connect_shape() {
    assert_eq!(
        connect::encode_end_stream().as_ref(),
        &[2, 0, 0, 0, 2, b'{', b'}']
    );
}

#[test]
fn error_end_stream_is_flagged_json_not_protobuf() {
    let frame = connect::encode_error_end_stream(&connect::ConnectStreamError {
        code: connect::ConnectCode::Unavailable,
        message: "overloaded".into(),
        details: vec![connect::ConnectErrorDetail {
            type_name: "aiserver.v1.ErrorDetails".into(),
            value: "AQ".into(),
        }],
    })
    .unwrap();
    let (flags, payload) = connect::decode_frames(&frame).unwrap().pop().unwrap();
    assert_eq!(flags, connect::END_STREAM_FLAG);
    let json: serde_json::Value = serde_json::from_slice(&payload).unwrap();
    assert_eq!(json["error"]["code"], "unavailable");
    assert_eq!(json["error"]["message"], "overloaded");
    assert_eq!(
        json["error"]["details"][0]["type"],
        "aiserver.v1.ErrorDetails"
    );
}

#[test]
fn cursor_error_details_subset_decodes_captured_wire_value() {
    let captured = "CAISVQoUQXV0aGVudGljYXRpb24gZXJyb3ISMklmIHlvdSBhcmUgbG9nZ2VkIGluLCB0cnkgbG9nZ2luZyBvdXQgYW5kIGJhY2sgaW4uIABSBwoFbG9naW4YAQ";
    let bytes = STANDARD_NO_PAD.decode(captured).unwrap();
    let details = ai::ErrorDetails::decode(bytes.as_slice()).unwrap();
    assert_eq!(details.error, 2, "ERROR_NOT_LOGGED_IN");
    assert_eq!(details.is_expected, Some(true));
    let custom = details.details.unwrap();
    assert_eq!(custom.title, "Authentication error");
    assert_eq!(custom.is_retryable, Some(false));
}

#[test]
fn captured_kv_ack_hex_decodes_as_agent_client_message() {
    let bytes = hex::decode("1a0408011a00").unwrap();
    let message = pb::AgentClientMessage::decode(bytes.as_slice()).unwrap();
    let Some(pb::agent_client_message::Message::KvClientMessage(kv)) = message.message else {
        panic!("expected KV client message")
    };
    assert_eq!(kv.id, 1);
    assert!(matches!(
        kv.message,
        Some(pb::kv_client_message::Message::SetBlobResult(_))
    ));
}

#[tokio::test]
async fn bidi_append_gzip_body_is_decompressed_before_protobuf_decode() {
    let (_directory, store) = fixtures::temp_store().await;
    let assets = PromptAssets::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("prompt/cursor")
            .as_path(),
    )
    .unwrap();
    let clients = NetworkClients::new(store.clone());
    let registry = TransportRegistry::new(
        store,
        Arc::new(fake_provider::FakeProvider::default()),
        PromptCompiler::new(assets),
    );
    let wire = ai::BidiAppendRequest {
        request_id: Some(ai::BidiRequestId {
            request_id: "gzip-request".into(),
        }),
        ..Default::default()
    }
    .encode_to_vec();
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&wire).unwrap();
    let compressed = encoder.finish().unwrap();

    let response = cursor::router(registry, clients)
        .unwrap()
        .oneshot(
            Request::post("/aiserver.v1.BidiService/BidiAppend")
                .header(header::CONTENT_TYPE, "application/proto")
                .header(header::CONTENT_ENCODING, "gzip")
                .body(Body::from(compressed))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let text = std::str::from_utf8(&body).unwrap();
    assert!(text.contains("BidiAppend contains no AgentClientMessage"));
    assert!(!text.contains("protobuf decode error"));
}

#[tokio::test]
async fn subagent_http_accepts_independent_parent_headers_and_replay() {
    let (_directory, store) = fixtures::temp_store().await;
    let assets = PromptAssets::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("prompt/cursor")
            .as_path(),
    )
    .unwrap();
    let registry = TransportRegistry::new(
        store.clone(),
        Arc::new(fake_provider::FakeProvider::default()),
        PromptCompiler::new(assets),
    );
    let app = cursor::router(registry.clone(), NetworkClients::new(store.clone())).unwrap();
    for (id, request_id, tool_id) in [
        ("tool-only", None, Some("task-1")),
        ("request-only", Some("parent-1"), None),
        ("both", Some("parent-2"), Some("task-2")),
    ] {
        let handle = registry.get_or_create(id).await.unwrap();
        let message = pb::AgentClientMessage {
            message: Some(pb::agent_client_message::Message::ClientHeartbeat(
                pb::ClientHeartbeat::default(),
            )),
        };
        let wire = ai::BidiAppendRequest {
            request_id: Some(ai::BidiRequestId {
                request_id: id.into(),
            }),
            data: hex::encode(message.encode_to_vec()),
            ..Default::default()
        };
        for _ in 0..2 {
            let mut request = Request::post("/aiserver.v1.BidiService/BidiAppend");
            if let Some(id) = request_id {
                request = request.header("x-parent-request-id", id);
            }
            if let Some(id) = tool_id {
                request = request.header("x-parent-agent-tool-call-id", id);
            }
            let response = app
                .clone()
                .oneshot(request.body(Body::from(wire.encode_to_vec())).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }
        let parent = handle.parent().unwrap();
        assert_eq!(parent.request_id.as_deref(), request_id);
        assert_eq!(parent.tool_call_id.as_deref(), tool_id);
        // Late lineage may fill a missing field, but must never replace an ID.
        if request_id.is_none() {
            handle
                .set_parent(cursor_server::cursor::TransportParent {
                    request_id: Some("late-parent".into()),
                    tool_call_id: tool_id.map(str::to_owned),
                })
                .unwrap();
            assert_eq!(
                handle.parent().unwrap().request_id.as_deref(),
                Some("late-parent")
            );
        }
        assert!(handle
            .set_parent(cursor_server::cursor::TransportParent {
                request_id: Some("conflict".into()),
                tool_call_id: Some("conflict".into())
            })
            .is_err());
    }
    registry.shutdown().await;
}

#[tokio::test]
async fn rejected_initial_append_terminates_waiting_stream_with_original_error_stage() {
    let (_directory, store) = fixtures::temp_store().await;
    let assets = PromptAssets::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("prompt/cursor")
            .as_path(),
    )
    .unwrap();
    let registry = TransportRegistry::new(
        store.clone(),
        Arc::new(fake_provider::FakeProvider::default()),
        PromptCompiler::new(assets),
    );
    let app = cursor::router(registry.clone(), NetworkClients::new(store.clone())).unwrap();
    let stream_app = app.clone();
    let pending = tokio::spawn(async move {
        stream_app
            .oneshot(
                Request::post("/agent.v1.AgentService/RunSSE")
                    .body(Body::from(
                        pb::BidiRequestId {
                            request_id: "rejected-child".into(),
                        }
                        .encode_to_vec(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap()
    });
    let bad = ai::BidiAppendRequest {
        request_id: Some(ai::BidiRequestId {
            request_id: "rejected-child".into(),
        }),
        ..Default::default()
    };
    let rejected = app
        .oneshot(
            Request::post("/aiserver.v1.BidiService/BidiAppend")
                .body(Body::from(bad.encode_to_vec()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status(), 400);
    let response = tokio::time::timeout(std::time::Duration::from_secs(2), pending)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response.status(), 400);
    assert!(registry.local("rejected-child").await.is_none());
    let diagnostics = store
        .route_diagnostics(Some("rejected-child"), None)
        .await
        .unwrap();
    assert_eq!(diagnostics.len(), 2);
    assert!(diagnostics
        .iter()
        .any(|d| d.stage == "decode" && d.http_status == 400 && d.path.ends_with("BidiAppend")));
    assert!(diagnostics
        .iter()
        .any(|d| d.stage == "route_lookup" && d.http_status == 400));
}

#[tokio::test]
async fn invalid_initial_metadata_cannot_publish_an_idle_transport() {
    let (_directory, store) = fixtures::temp_store().await;
    let assets =
        PromptAssets::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("prompt/cursor"))
            .unwrap();
    let registry = TransportRegistry::new(
        store.clone(),
        Arc::new(fake_provider::FakeProvider::default()),
        PromptCompiler::new(assets),
    );
    let app = cursor::router(registry.clone(), NetworkClients::new(store)).unwrap();
    for empty_parent in [true, false] {
        let id = if empty_parent {
            "empty-parent"
        } else {
            "empty-conversation"
        };
        let message = pb::AgentClientMessage {
            message: Some(pb::agent_client_message::Message::RunRequest(
                pb::AgentRunRequest {
                    conversation_id: Some(if empty_parent { "valid" } else { "" }.into()),
                    requested_model: Some(pb::RequestedModel {
                        model_id: "plugin:synthetic/model".into(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )),
        };
        let wire = ai::BidiAppendRequest {
            request_id: Some(ai::BidiRequestId {
                request_id: id.into(),
            }),
            data: hex::encode(message.encode_to_vec()),
            ..Default::default()
        };
        let mut request = Request::post("/aiserver.v1.BidiService/BidiAppend");
        if empty_parent {
            request = request.header("x-parent-agent-tool-call-id", "");
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::from(wire.encode_to_vec())).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), 400);
        assert!(registry.local(id).await.is_none());
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            app.clone().oneshot(
                Request::post("/agent.v1.AgentService/RunSSE")
                    .body(Body::from(
                        pb::BidiRequestId {
                            request_id: id.into(),
                        }
                        .encode_to_vec(),
                    ))
                    .unwrap(),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(response.status(), 400);
    }
    registry.shutdown().await;
}

#[tokio::test]
async fn checkpoint_uses_tool_id_that_arrived_after_builder_creation() {
    use cursor_server::{
        cursor::{
            checkpoint::{CheckpointBuilder, PendingSteps},
            services::blob_sync::BlobSynchronizer,
            TransportParent,
        },
        model::{CanonicalMessage, MessageContent, Origin, Role, ToolCallContent},
    };
    let (_directory, store) = fixtures::temp_store().await;
    let assets =
        PromptAssets::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("prompt/cursor"))
            .unwrap();
    let registry = TransportRegistry::new(
        store.clone(),
        Arc::new(fake_provider::FakeProvider::default()),
        PromptCompiler::new(assets),
    );
    let handle = registry.get_or_create("late-lineage").await.unwrap();
    handle
        .set_parent(TransportParent {
            request_id: Some("parent".into()),
            tool_call_id: None,
        })
        .unwrap();
    let sync = BlobSynchronizer::new("late-lineage".into(), store.clone(), handle.clone());
    let mut output = handle.subscribe();
    let acknowledger = sync.clone();
    let task = tokio::spawn(async move {
        while let Some(frame) = output.recv().await {
            for (flags, payload) in connect::decode_frames(&frame).unwrap() {
                if flags & connect::END_STREAM_FLAG != 0 {
                    return;
                }
                if let Some(pb::agent_server_message::Message::KvServerMessage(kv)) =
                    pb::AgentServerMessage::decode(payload).unwrap().message
                {
                    acknowledger
                        .handle_client(pb::KvClientMessage {
                            id: kv.id,
                            message: Some(pb::kv_client_message::Message::SetBlobResult(
                                pb::SetBlobResult { error: None },
                            )),
                        })
                        .await
                        .unwrap();
                }
            }
        }
    });
    let mut builder = CheckpointBuilder::new(store, sync, None, None);
    builder.follow_parent(handle.clone());
    handle
        .set_parent(TransportParent {
            request_id: None,
            tool_call_id: Some("late-task".into()),
        })
        .unwrap();
    let message = CanonicalMessage {
        message_id: "progress".into(),
        role: Role::Assistant,
        origin: Origin::Assistant,
        runtime_event_id: None,
        content: MessageContent::Assistant {
            text: String::new(),
            thinking: String::new(),
            tool_round_id: None,
            replay_state: None,
            tool_calls: vec![ToolCallContent {
                index: 0,
                call_id: "update".into(),
                name: "UpdateCurrentStep".into(),
                arguments: serde_json::json!({"final_summary":"finished synthetic work"}),
            }],
        },
    };
    let checkpoint = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        builder.settled(
            &[message],
            pb::AgentMode::Agent as i32,
            &PendingSteps::default(),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        checkpoint.communicate_update_states_by_parent_tool_call_id["late-task"]
            .final_summary
            .as_deref(),
        Some("finished synthetic work")
    );
    registry.shutdown().await;
    task.await.unwrap();
}
