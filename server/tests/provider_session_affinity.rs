//! Real loopback HTTP assertions; no real model, credentials or Cursor needed.
use axum::{extract::State, routing::post, Json, Router};
use cursor_server::{
    config::{ProviderConfig, ProviderKind},
    model::{ModelInvocation, ModelRequest, ModelSpec, PromptSpec},
    provider::{OpenAiChatProvider, OpenAiResponsesProvider, Provider},
};
use futures_util::StreamExt;
use serde_json::Value;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn gpt_affinity_survives_turns_but_isolates_conversations_for_both_protocols() {
    let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
    let router = Router::new().route("/responses", post(|State(state): State<Arc<Mutex<Vec<Value>>>>, Json(body): Json<Value>| async move {
        state.lock().unwrap().push(body);
        ([ ("content-type", "text/event-stream") ], "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"fixture\",\"output\":[]}}\n\n")
    })).route("/chat", post(|State(state): State<Arc<Mutex<Vec<Value>>>>, Json(body): Json<Value>| async move {
        state.lock().unwrap().push(body);
        ([ ("content-type", "text/event-stream") ], "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n")
    })).with_state(captured.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    for responses in [true, false] {
        let config = ProviderConfig {
            kind: if responses {
                ProviderKind::OpenAiResponses
            } else {
                ProviderKind::OpenAiChat
            },
            request_url: format!(
                "http://{addr}/{}",
                if responses { "responses" } else { "chat" }
            ),
            api_key: "synthetic".into(),
            custom_headers: Default::default(),
            max_output_tokens: None,
            request_timeout: Duration::from_secs(5),
            allowed_body_fields: None,
        };
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let provider: Box<dyn Provider> = if responses {
            Box::new(OpenAiResponsesProvider::new(client, config))
        } else {
            Box::new(OpenAiChatProvider::new(client, config))
        };
        for (index, conversation) in ["parent", "parent", "child"].iter().enumerate() {
            let invocation = ModelInvocation {
                call_id: format!("call-{index}"),
                run_id: format!("run-{index}"),
                conversation_id: conversation.to_string(),
                provider_call_index: index as u64,
                request: ModelRequest {
                    model: ModelSpec::new("gpt-fixture"),
                    prompt: PromptSpec {
                        instructions: String::new(),
                        tools: vec![],
                    },
                    history: vec![],
                },
            };
            let mut stream = provider.stream(invocation, CancellationToken::new());
            while let Some(event) = stream.next().await {
                event.unwrap();
            }
        }
    }
    let bodies = captured.lock().unwrap();
    assert_eq!(bodies.len(), 6);
    for chunk in bodies.chunks(3) {
        assert_eq!(chunk[0]["prompt_cache_key"], chunk[1]["prompt_cache_key"]);
        assert_ne!(chunk[0]["prompt_cache_key"], chunk[2]["prompt_cache_key"]);
        let key = chunk[0]["prompt_cache_key"].as_str().unwrap();
        assert!(key.len() <= 64 && key != "cursor-byok" && !key.contains("parent"));
    }
    assert_eq!(bodies[0]["prompt_cache_key"], bodies[3]["prompt_cache_key"]);
    drop(bodies);
    server.abort();
}
