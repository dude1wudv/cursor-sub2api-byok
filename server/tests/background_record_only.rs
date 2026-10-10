//! Record-only background completion updates checkpoints without waking model runs.
#[path = "support/fake_provider.rs"]
mod fake_provider;
#[path = "support/fixtures.rs"]
mod fixtures;

use std::{collections::HashMap, sync::Arc, time::Duration};

use bytes::Bytes;
use cursor_server::{
    cursor::{
        prompting::{PromptAssets, PromptCompiler},
        protocol::{connect, proto::agent::v1 as pb},
        TransportCommand, TransportHandle, TransportRegistry,
    },
    model::{ProjectedContent, Role},
    provider::{FinishReason, ModelEvent},
};
use prost::Message;
use tokio::sync::mpsc::UnboundedReceiver;

#[test]
fn completion_timestamp_and_record_only_use_the_captured_protobuf_tags() {
    let completion = pb::BackgroundTaskCompletion {
        completed_at_ms: Some(150),
        record_only: true,
        ..Default::default()
    };

    // Field 12: optional uint64 (0x60), field 13: bool (0x68).
    let wire = completion.encode_to_vec();
    assert_eq!(wire, [0x60, 0x96, 0x01, 0x68, 0x01]);
    let decoded = pb::BackgroundTaskCompletion::decode(wire.as_slice()).unwrap();
    assert_eq!(decoded.completed_at_ms, Some(150));
    assert!(decoded.record_only);
}

#[tokio::test]
async fn standalone_record_only_shell_returns_the_existing_checkpoint_and_closes() {
    let (_directory, store) = fixtures::temp_store().await;
    let provider = fake_provider::FakeProvider::default();
    let registry = registry(store, provider.clone());
    let handle = registry.get_or_create("record-only-shell").await.unwrap();
    let original = preserved_state();

    let (checkpoint, exec_messages, closed) = run_record_only(
        &handle,
        background_run(
            "shell-record-only-run",
            original.clone(),
            vec![completion(
                "shell-record-only",
                pb::BackgroundTaskKind::Shell,
                "shell-call",
                true,
                500,
            )],
        ),
    )
    .await;

    assert_eq!(
        checkpoint, original,
        "a record-only shell must not invent conversation history"
    );
    assert_eq!(
        exec_messages, 0,
        "record-only must not request model context"
    );
    assert!(
        closed,
        "the standalone record-only request must close successfully"
    );
    assert!(
        provider.requests().is_empty(),
        "record-only must not call a provider"
    );
}

#[tokio::test]
async fn standalone_record_only_subagent_updates_only_its_terminal_checkpoint_state() {
    let (_directory, store) = fixtures::temp_store().await;
    let provider = fake_provider::FakeProvider::default();
    let registry = registry(store, provider.clone());
    let handle = registry
        .get_or_create("record-only-subagent")
        .await
        .unwrap();
    let mut original = preserved_state();
    original.subagent_states.insert(
        "agent-7".into(),
        pb::SubagentPersistedState {
            environment: pb::SubagentExecutionEnvironment::Local as i32,
            ..Default::default()
        },
    );
    original.subagent_runs_by_parent_tool_call_id.insert(
        "shell-call-existing".into(),
        pb::SubagentRunState {
            parent_tool_call_id: "shell-call-existing".into(),
            subagent_id: Some("existing-agent".into()),
            status: pb::SubagentRunStatus::Running as i32,
            title: Some("Still running".into()),
            ..Default::default()
        },
    );
    let before_turns = original.turns.clone();
    let before_root = original.root_prompt_messages_json.clone();
    let before_pending = original.pending_tool_calls.clone();

    let (checkpoint, exec_messages, closed) = run_record_only(
        &handle,
        background_run(
            "record-only-subagent-run",
            original,
            vec![completion(
                "agent-7",
                pb::BackgroundTaskKind::Subagent,
                "agent-tool-7",
                true,
                700,
            )],
        ),
    )
    .await;

    assert_eq!(checkpoint.turns, before_turns);
    assert_eq!(checkpoint.root_prompt_messages_json, before_root);
    assert_eq!(checkpoint.pending_tool_calls, before_pending);
    assert_eq!(checkpoint.mode, Some(pb::AgentMode::Multitask as i32));
    assert_eq!(checkpoint.is_root_project_conversation, Some(true));
    assert_eq!(checkpoint.subagent_runs_by_parent_tool_call_id.len(), 2);
    let recorded = &checkpoint.subagent_runs_by_parent_tool_call_id["agent-tool-7"];
    assert_eq!(recorded.subagent_id.as_deref(), Some("agent-7"));
    assert_eq!(recorded.status, pb::SubagentRunStatus::Success as i32);
    assert_eq!(recorded.completed_timestamp_ms, Some(700));
    assert_eq!(
        recorded.completion_reason,
        Some(pb::BackgroundTaskCompletionReason::TaskFinished as i32)
    );
    assert_eq!(
        recorded.environment,
        pb::SubagentExecutionEnvironment::Local as i32
    );
    assert_eq!(exec_messages, 0);
    assert!(closed);
    assert!(provider.requests().is_empty());
}

#[tokio::test]
async fn mixed_completion_wakes_with_only_non_record_only_items() {
    let (_directory, store) = fixtures::temp_store().await;
    let provider = fake_provider::FakeProvider::default();
    provider.push(stop_response(
        "mixed-completion",
        "handled ordinary completion",
    ));
    let registry = registry(store, provider.clone());
    let handle = registry.get_or_create("mixed-record-only").await.unwrap();

    let (checkpoint, _blobs) = drive_completion(
        &handle,
        background_run(
            "mixed-completion-run",
            pb::ConversationStateStructure {
                mode: Some(pb::AgentMode::Multitask as i32),
                ..Default::default()
            },
            vec![
                completion(
                    "private-record-only-marker",
                    pb::BackgroundTaskKind::Subagent,
                    "private-record-tool",
                    true,
                    810,
                ),
                completion(
                    "ordinary-completion-marker",
                    pb::BackgroundTaskKind::Subagent,
                    "ordinary-completion-tool",
                    false,
                    820,
                ),
            ],
        ),
    )
    .await;

    let requests = provider.requests();
    assert_eq!(requests.len(), 1);
    let [runtime] = requests[0].history.as_slice() else {
        panic!("mixed completion must add one runtime message")
    };
    assert_eq!(runtime.role, Role::User);
    let ProjectedContent::Parts(parts) = &runtime.content else {
        panic!("mixed completion context must be text")
    };
    let text = parts
        .iter()
        .filter_map(|part| match part {
            cursor_server::model::ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert!(text.contains("ordinary-completion-marker"));
    assert!(text.contains("ordinary-completion-tool"));
    assert!(!text.contains("private-record-only-marker"));
    assert!(!text.contains("private-record-tool"));
    assert_eq!(
        checkpoint.subagent_runs_by_parent_tool_call_id["private-record-tool"].status,
        pb::SubagentRunStatus::Success as i32
    );
}

#[tokio::test]
async fn runtime_record_only_action_keeps_active_run_and_updates_its_final_checkpoint() {
    let (_directory, store) = fixtures::temp_store().await;
    let provider = fake_provider::FakeProvider::default();
    let _release_provider = provider.push_gated(stop_response("active-run", "active run finished"));
    let registry = registry(store, provider.clone());
    let handle = registry.get_or_create("active-record-only").await.unwrap();
    let mut output = handle.subscribe();
    handle
        .command(TransportCommand::Append {
            seqno: 0,
            message: Box::new(user_run(
                "active-user-run",
                pb::ConversationStateStructure {
                    mode: Some(pb::AgentMode::Multitask as i32),
                    ..Default::default()
                },
            )),
        })
        .await
        .unwrap();
    let (mut append_seqno, mut blobs) =
        start_run_until_provider(&handle, &mut output, &provider).await;
    assert_eq!(provider.requests().len(), 1);

    handle
        .command(TransportCommand::Append {
            seqno: append_seqno,
            message: Box::new(runtime_record_only_action(completion(
                "active-child",
                pb::BackgroundTaskKind::Subagent,
                "active-child-tool",
                true,
                900,
            ))),
        })
        .await
        .unwrap();
    append_seqno += 1;

    let (mut last_checkpoint, next_seqno) =
        wait_for_terminal_checkpoint(&handle, &mut output, append_seqno, "active-child-tool").await;
    assert_eq!(
        provider.requests().len(),
        1,
        "record-only must not start another provider call"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(50), output.recv())
            .await
            .is_err(),
        "record-only must not cancel or close the active run"
    );
    handle
        .command(TransportCommand::Append {
            seqno: next_seqno,
            message: Box::new(runtime_cancel_action()),
        })
        .await
        .unwrap();
    let end = collect_after_cancel(
        &handle,
        output,
        next_seqno + 1,
        &mut blobs,
        &mut last_checkpoint,
    )
    .await;
    assert_eq!(end["error"]["code"], "canceled");

    assert_eq!(provider.requests().len(), 1);
    let child = &last_checkpoint.subagent_runs_by_parent_tool_call_id["active-child-tool"];
    assert_eq!(child.status, pb::SubagentRunStatus::Success as i32);
    assert_eq!(child.completed_timestamp_ms, Some(900));
}

#[tokio::test]
async fn older_record_only_completion_does_not_regress_a_terminal_child() {
    let (_directory, store) = fixtures::temp_store().await;
    let provider = fake_provider::FakeProvider::default();
    let registry = registry(store, provider.clone());
    let handle = registry
        .get_or_create("old-record-only-timestamp")
        .await
        .unwrap();
    let mut original = preserved_state();
    original.subagent_runs_by_parent_tool_call_id.insert(
        "stable-tool".into(),
        pb::SubagentRunState {
            parent_tool_call_id: "stable-tool".into(),
            subagent_id: Some("stable-child".into()),
            status: pb::SubagentRunStatus::Error as i32,
            title: Some("Newer terminal title".into()),
            detail: Some("Newer terminal detail".into()),
            output_path: Some("/tmp/newer-result.json".into()),
            completed_timestamp_ms: Some(1_000),
            completion_reason: Some(pb::BackgroundTaskCompletionReason::TaskFinished as i32),
            ..Default::default()
        },
    );
    let original_run = original.subagent_runs_by_parent_tool_call_id["stable-tool"].clone();

    let (checkpoint, exec_messages, closed) = run_record_only(
        &handle,
        background_run(
            "older-record-only-run",
            original,
            vec![completion(
                "stable-child",
                pb::BackgroundTaskKind::Subagent,
                "stable-tool",
                true,
                999,
            )],
        ),
    )
    .await;

    assert_eq!(
        checkpoint.subagent_runs_by_parent_tool_call_id["stable-tool"],
        original_run
    );
    assert_eq!(exec_messages, 0);
    assert!(closed);
    assert!(provider.requests().is_empty());
}

#[tokio::test]
async fn empty_completion_array_still_returns_an_explicit_protocol_error() {
    let (_directory, store) = fixtures::temp_store().await;
    let provider = fake_provider::FakeProvider::default();
    let registry = registry(store, provider.clone());
    let handle = registry
        .get_or_create("empty-completion-action")
        .await
        .unwrap();
    let mut output = handle.subscribe();
    handle
        .command(TransportCommand::Append {
            seqno: 0,
            message: Box::new(background_run(
                "empty-completion-run",
                pb::ConversationStateStructure {
                    mode: Some(pb::AgentMode::Multitask as i32),
                    ..Default::default()
                },
                Vec::new(),
            )),
        })
        .await
        .unwrap();

    let mut exec_messages = 0;
    let mut append_seqno = 1;
    let error = loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), output.recv())
            .await
            .unwrap()
            .unwrap();
        let (flags, payload) = connect::decode_frames(&frame).unwrap().pop().unwrap();
        if flags & connect::END_STREAM_FLAG != 0 {
            let body: serde_json::Value = serde_json::from_slice(&payload).unwrap();
            break body.get("error").cloned();
        }
        let server = pb::AgentServerMessage::decode(payload).unwrap();
        if let Some(pb::agent_server_message::Message::ExecServerMessage(exec)) = server.message {
            assert!(matches!(
                exec.message,
                Some(pb::exec_server_message::Message::RequestContextArgs(_))
            ));
            exec_messages += 1;
            append_seqno = answer_context(&handle, exec.id, append_seqno).await;
        }
    };

    assert_eq!(
        error.as_ref().and_then(|error| error["code"].as_str()),
        Some("invalid_argument")
    );
    assert_eq!(
        error.as_ref().and_then(|error| error["message"].as_str()),
        Some("background task completion action contains no completion")
    );
    assert_eq!(exec_messages, 1);
    assert!(provider.requests().is_empty());
}

fn registry(
    store: cursor_server::store::Store,
    provider: fake_provider::FakeProvider,
) -> TransportRegistry {
    let assets = PromptAssets::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("prompt/cursor")
            .as_path(),
    )
    .unwrap();
    TransportRegistry::new(store, Arc::new(provider), PromptCompiler::new(assets))
}

fn preserved_state() -> pb::ConversationStateStructure {
    pb::ConversationStateStructure {
        root_prompt_messages_json: vec![b"root-message-marker".to_vec()],
        turns: vec![b"turn-marker".to_vec()],
        pending_tool_calls: vec!["pending-marker".into()],
        mode: Some(pb::AgentMode::Multitask as i32),
        todos: vec![b"todo-marker".to_vec()],
        summary: Some(b"summary-marker".to_vec()),
        is_root_project_conversation: Some(true),
        ..Default::default()
    }
}

fn completion(
    task_id: &str,
    kind: pb::BackgroundTaskKind,
    tool_call_id: &str,
    record_only: bool,
    completed_at_ms: u64,
) -> pb::BackgroundTaskCompletion {
    pb::BackgroundTaskCompletion {
        task_id: task_id.into(),
        kind: kind as i32,
        status: pb::BackgroundTaskStatus::Success as i32,
        title: format!("title-{task_id}"),
        detail: Some(format!("detail-{task_id}")),
        output_path: Some(format!("/tmp/{task_id}.json")),
        reason: pb::BackgroundTaskCompletionReason::TaskFinished as i32,
        subagent_id: (kind == pb::BackgroundTaskKind::Subagent).then(|| task_id.into()),
        tool_call_id: Some(tool_call_id.into()),
        completed_at_ms: Some(completed_at_ms),
        record_only,
        ..Default::default()
    }
}

fn background_run(
    run_id: &str,
    state: pb::ConversationStateStructure,
    completions: Vec<pb::BackgroundTaskCompletion>,
) -> pb::AgentClientMessage {
    pb::AgentClientMessage {
        message: Some(pb::agent_client_message::Message::RunRequest(
            pb::AgentRunRequest {
                action: Some(pb::ConversationAction {
                    action: Some(
                        pb::conversation_action::Action::BackgroundTaskCompletionAction(
                            pb::BackgroundTaskCompletionAction { completions },
                        ),
                    ),
                    ..Default::default()
                }),
                conversation_id: Some("record-only-conversation".into()),
                requested_model: Some(pb::RequestedModel {
                    model_id: "test-model".into(),
                    ..Default::default()
                }),
                conversation_state: Some(state),
                run_id: Some(run_id.into()),
                ..Default::default()
            },
        )),
    }
}

fn user_run(run_id: &str, state: pb::ConversationStateStructure) -> pb::AgentClientMessage {
    pb::AgentClientMessage {
        message: Some(pb::agent_client_message::Message::RunRequest(
            pb::AgentRunRequest {
                action: Some(pb::ConversationAction {
                    action: Some(pb::conversation_action::Action::UserMessageAction(
                        pb::UserMessageAction {
                            user_message: Some(pb::UserMessage {
                                text: "continue the active test run".into(),
                                message_id: "active-user-message".into(),
                                mode: pb::AgentMode::Multitask as i32,
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                    )),
                    ..Default::default()
                }),
                conversation_id: Some("record-only-conversation".into()),
                requested_model: Some(pb::RequestedModel {
                    model_id: "test-model".into(),
                    ..Default::default()
                }),
                conversation_state: Some(state),
                run_id: Some(run_id.into()),
                ..Default::default()
            },
        )),
    }
}

fn runtime_record_only_action(completion: pb::BackgroundTaskCompletion) -> pb::AgentClientMessage {
    pb::AgentClientMessage {
        message: Some(pb::agent_client_message::Message::ConversationAction(
            pb::ConversationAction {
                action: Some(
                    pb::conversation_action::Action::BackgroundTaskCompletionAction(
                        pb::BackgroundTaskCompletionAction {
                            completions: vec![completion],
                        },
                    ),
                ),
                ..Default::default()
            },
        )),
    }
}

async fn run_record_only(
    handle: &TransportHandle,
    message: pb::AgentClientMessage,
) -> (pb::ConversationStateStructure, usize, bool) {
    let mut output = handle.subscribe();
    handle
        .command(TransportCommand::Append {
            seqno: 0,
            message: Box::new(message),
        })
        .await
        .unwrap();
    let mut checkpoint = None;
    let mut exec_messages = 0;
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), output.recv())
            .await
            .unwrap()
            .expect("record-only output stream closed before its terminal frame");
        let (flags, payload) = connect::decode_frames(&frame).unwrap().pop().unwrap();
        if flags & connect::END_STREAM_FLAG != 0 {
            assert_eq!(payload.as_ref(), b"{}");
            assert!(tokio::time::timeout(Duration::from_secs(1), output.recv())
                .await
                .unwrap()
                .is_none());
            return (
                checkpoint.expect("record-only request must return checkpoint state"),
                exec_messages,
                true,
            );
        }
        let server = pb::AgentServerMessage::decode(payload).unwrap();
        match server.message {
            Some(pb::agent_server_message::Message::ConversationCheckpointUpdate(state)) => {
                checkpoint = Some(state);
            }
            Some(pb::agent_server_message::Message::ExecServerMessage(_)) => {
                exec_messages += 1;
            }
            Some(pb::agent_server_message::Message::KvServerMessage(kv)) => {
                panic!(
                    "record-only request unexpectedly emitted KV message {}",
                    kv.id
                )
            }
            _ => {}
        }
    }
}

async fn drive_completion(
    handle: &TransportHandle,
    message: pb::AgentClientMessage,
) -> (pb::ConversationStateStructure, HashMap<Vec<u8>, Vec<u8>>) {
    let output = handle.subscribe();
    handle
        .command(TransportCommand::Append {
            seqno: 0,
            message: Box::new(message),
        })
        .await
        .unwrap();
    finish_active_run(handle, output, 1, &mut HashMap::new())
        .await
        .into_state_and_blobs()
}

struct FinishedRun {
    state: pb::ConversationStateStructure,
    blobs: HashMap<Vec<u8>, Vec<u8>>,
}

impl FinishedRun {
    fn into_state_and_blobs(self) -> (pb::ConversationStateStructure, HashMap<Vec<u8>, Vec<u8>>) {
        (self.state, self.blobs)
    }
}

async fn start_run_until_provider(
    handle: &TransportHandle,
    output: &mut UnboundedReceiver<Bytes>,
    provider: &fake_provider::FakeProvider,
) -> (i64, HashMap<Vec<u8>, Vec<u8>>) {
    let mut append_seqno = 1;
    let mut blobs = HashMap::new();
    let deadline = tokio::time::sleep(Duration::from_secs(5));
    tokio::pin!(deadline);
    let mut poll_requests = tokio::time::interval(Duration::from_millis(10));
    loop {
        tokio::select! {
            _ = &mut deadline => panic!("provider was not called before the deadline"),
            _ = poll_requests.tick() => {
                if !provider.requests().is_empty() {
                    break;
                }
            }
            frame = output.recv() => {
                let frame = frame.expect("active run output closed before provider start");
        let (flags, payload) = connect::decode_frames(&frame).unwrap().pop().unwrap();
        assert_eq!(
            flags & connect::END_STREAM_FLAG,
            0,
            "active run ended before provider start"
        );
        let server = pb::AgentServerMessage::decode(payload).unwrap();
        match server.message {
            Some(pb::agent_server_message::Message::ExecServerMessage(exec)) => {
                assert!(matches!(
                    exec.message,
                    Some(pb::exec_server_message::Message::RequestContextArgs(_))
                ));
                append_seqno = answer_context(handle, exec.id, append_seqno).await;
            }
            Some(pb::agent_server_message::Message::KvServerMessage(kv)) => {
                blobs.insert(kv.id.to_be_bytes().to_vec(), kv.id.to_be_bytes().to_vec());
                handle
                    .command(TransportCommand::Append {
                        seqno: append_seqno,
                        message: Box::new(kv_ack(kv.id)),
                    })
                    .await
                    .unwrap();
                append_seqno += 1;
            }
            Some(pb::agent_server_message::Message::ConversationCheckpointUpdate(_)) => {}
            _ => {}
        }
            }
        }
    }
    (append_seqno, blobs)
}

async fn answer_context(handle: &TransportHandle, id: u32, mut seqno: i64) -> i64 {
    handle
        .command(TransportCommand::Append {
            seqno,
            message: Box::new(pb::AgentClientMessage {
                message: Some(pb::agent_client_message::Message::ExecClientControlMessage(
                    pb::ExecClientControlMessage {
                        message: Some(pb::exec_client_control_message::Message::StreamClose(
                            pb::ExecClientStreamClose { id },
                        )),
                    },
                )),
            }),
        })
        .await
        .unwrap();
    seqno += 1;
    handle
        .command(TransportCommand::Append {
            seqno,
            message: Box::new(pb::AgentClientMessage {
                message: Some(pb::agent_client_message::Message::ExecClientMessage(
                    pb::ExecClientMessage {
                        id,
                        message: Some(pb::exec_client_message::Message::RequestContextResult(
                            pb::RequestContextResult {
                                result: Some(pb::request_context_result::Result::Success(
                                    pb::RequestContextSuccess {
                                        request_context: Some(pb::RequestContext::default()),
                                        ..Default::default()
                                    },
                                )),
                            },
                        )),
                        ..Default::default()
                    },
                )),
            }),
        })
        .await
        .unwrap();
    seqno + 1
}

async fn finish_active_run(
    handle: &TransportHandle,
    mut output: UnboundedReceiver<Bytes>,
    mut append_seqno: i64,
    blobs: &mut HashMap<Vec<u8>, Vec<u8>>,
) -> FinishedRun {
    let mut final_checkpoint = None;
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), output.recv())
            .await
            .unwrap()
            .expect("active run ended without an end-stream frame");
        let (flags, payload) = connect::decode_frames(&frame).unwrap().pop().unwrap();
        if flags & connect::END_STREAM_FLAG != 0 {
            assert_eq!(payload.as_ref(), b"{}");
            return FinishedRun {
                state: final_checkpoint.expect("active run must publish its settled checkpoint"),
                blobs: std::mem::take(blobs),
            };
        }
        let server = pb::AgentServerMessage::decode(payload).unwrap();
        match server.message {
            Some(pb::agent_server_message::Message::ExecServerMessage(exec)) => {
                assert!(matches!(
                    exec.message,
                    Some(pb::exec_server_message::Message::RequestContextArgs(_))
                ));
                append_seqno = answer_context(handle, exec.id, append_seqno).await;
            }
            Some(pb::agent_server_message::Message::KvServerMessage(kv)) => {
                handle
                    .command(TransportCommand::Append {
                        seqno: append_seqno,
                        message: Box::new(kv_ack(kv.id)),
                    })
                    .await
                    .unwrap();
                append_seqno += 1;
            }
            Some(pb::agent_server_message::Message::ConversationCheckpointUpdate(state))
                if state.pending_tool_calls.is_empty() =>
            {
                final_checkpoint = Some(state);
            }
            _ => {}
        }
    }
}

async fn wait_for_terminal_checkpoint(
    handle: &TransportHandle,
    output: &mut UnboundedReceiver<Bytes>,
    mut append_seqno: i64,
    tool_call_id: &str,
) -> (pb::ConversationStateStructure, i64) {
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), output.recv())
            .await
            .unwrap()
            .expect("active run ended before publishing the record-only checkpoint");
        let (flags, payload) = connect::decode_frames(&frame).unwrap().pop().unwrap();
        assert_eq!(
            flags & connect::END_STREAM_FLAG,
            0,
            "record-only ended the active run"
        );
        let server = pb::AgentServerMessage::decode(payload).unwrap();
        match server.message {
            Some(pb::agent_server_message::Message::ConversationCheckpointUpdate(state)) => {
                if let Some(child) = state.subagent_runs_by_parent_tool_call_id.get(tool_call_id) {
                    if matches!(
                        pb::SubagentRunStatus::try_from(child.status),
                        Ok(pb::SubagentRunStatus::Success
                            | pb::SubagentRunStatus::Error
                            | pb::SubagentRunStatus::Aborted)
                    ) {
                        return (state, append_seqno);
                    }
                }
            }
            Some(pb::agent_server_message::Message::KvServerMessage(kv)) => {
                handle
                    .command(TransportCommand::Append {
                        seqno: append_seqno,
                        message: Box::new(kv_ack(kv.id)),
                    })
                    .await
                    .unwrap();
                append_seqno += 1;
            }
            Some(pb::agent_server_message::Message::ExecServerMessage(exec)) => {
                panic!(
                    "unexpected Exec while recording background state: {:?}",
                    exec.message
                )
            }
            _ => {}
        }
    }
}

fn runtime_cancel_action() -> pb::AgentClientMessage {
    pb::AgentClientMessage {
        message: Some(pb::agent_client_message::Message::ConversationAction(
            pb::ConversationAction {
                action: Some(pb::conversation_action::Action::CancelAction(
                    pb::CancelAction::default(),
                )),
                ..Default::default()
            },
        )),
    }
}

async fn collect_after_cancel(
    handle: &TransportHandle,
    mut output: UnboundedReceiver<Bytes>,
    mut append_seqno: i64,
    blobs: &mut HashMap<Vec<u8>, Vec<u8>>,
    last_checkpoint: &mut pb::ConversationStateStructure,
) -> serde_json::Value {
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), output.recv())
            .await
            .unwrap()
            .expect("cancelled run did not close its output stream");
        let (flags, payload) = connect::decode_frames(&frame).unwrap().pop().unwrap();
        if flags & connect::END_STREAM_FLAG != 0 {
            return serde_json::from_slice(&payload).unwrap();
        }
        let server = pb::AgentServerMessage::decode(payload).unwrap();
        match server.message {
            Some(pb::agent_server_message::Message::ConversationCheckpointUpdate(state)) => {
                *last_checkpoint = state;
            }
            Some(pb::agent_server_message::Message::KvServerMessage(kv)) => {
                blobs.insert(kv.id.to_be_bytes().to_vec(), kv.id.to_be_bytes().to_vec());
                handle
                    .command(TransportCommand::Append {
                        seqno: append_seqno,
                        message: Box::new(kv_ack(kv.id)),
                    })
                    .await
                    .unwrap();
                append_seqno += 1;
            }
            Some(pb::agent_server_message::Message::ExecServerMessage(exec)) => {
                panic!(
                    "unexpected Exec after cancelling active run: {:?}",
                    exec.message
                )
            }
            _ => {}
        }
    }
}

fn kv_ack(id: u32) -> pb::AgentClientMessage {
    pb::AgentClientMessage {
        message: Some(pb::agent_client_message::Message::KvClientMessage(
            pb::KvClientMessage {
                id,
                message: Some(pb::kv_client_message::Message::SetBlobResult(
                    pb::SetBlobResult { error: None },
                )),
            },
        )),
    }
}

fn stop_response(model_call_id: &str, text: &str) -> Vec<ModelEvent> {
    vec![
        ModelEvent::Start {
            model_call_id: model_call_id.into(),
        },
        ModelEvent::TextStart,
        ModelEvent::TextDelta(text.into()),
        ModelEvent::TextEnd,
        ModelEvent::Done(FinishReason::Stop),
    ]
}
