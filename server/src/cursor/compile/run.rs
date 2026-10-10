//! Compiles an AgentRunRequest into a PreparedRun.
use std::collections::BTreeMap;

use uuid::Uuid;

use crate::{
    cursor::prompting::{Mode, PromptCompiler},
    cursor::{
        checkpoint::messages,
        checkpoint::CheckpointBuilder,
        protocol::proto::agent::v1 as pb,
        services::blob_sync::BlobSynchronizer,
        services::context_sync::RequestContextSynchronizer,
        tools::runtime::{ExecContext, SubagentModel},
    },
    model::{
        CanonicalMessage, ContentPart, ConversationId, MessageContent, Origin, PreparedRun,
        PromptSpec, Role, RunAction, RunId, RunKind,
    },
    store::{BlobId, Store},
    Error, Result,
};

use super::{break_messages, context, insert_messages, model};

struct ActionProjection {
    mode: i32,
    turn_user: Option<pb::UserMessage>,
    action_context: String,
    event_id: Option<String>,
    input_id: Option<String>,
    starts_turn: bool,
    compacting: bool,
    background_completion: bool,
}

pub struct CursorRunContext {
    pub request_id: String,
    pub mode: i32,
    pub turn_user: Option<pb::UserMessage>,
    pub exec: ExecContext,
    pub dynamic_tools: BTreeMap<String, pb::McpToolDefinition>,
    pub checkpoint_prompt: PromptSpec,
    pub compacting: bool,
    pub background_completion: bool,
}

pub(crate) struct PrepareDependencies<'a> {
    pub compiler: &'a PromptCompiler,
    pub store: &'a Store,
    pub checkpoint: &'a CheckpointBuilder,
    pub blob_sync: &'a BlobSynchronizer,
    pub context_sync: &'a RequestContextSynchronizer,
}

pub(crate) async fn prepare(
    request_id: &str,
    request: &pb::AgentRunRequest,
    dependencies: PrepareDependencies<'_>,
) -> Result<(PreparedRun, CursorRunContext)> {
    let PrepareDependencies {
        compiler,
        store,
        checkpoint,
        blob_sync,
        context_sync,
    } = dependencies;
    checkpoint
        .import_prefetched(&request.pre_fetched_blobs)
        .await?;
    let conversation_id = ConversationId::new(
        request
            .conversation_id
            .clone()
            .unwrap_or_else(|| request_id.into()),
    );
    let run_id = execution_run_id(request_id);
    let mut base_messages = if request.conversation_state.is_some() {
        Some(
            checkpoint
                .hydrate_messages(request.conversation_state.as_ref())
                .await?,
        )
    } else {
        None
    };
    if let Some(trace) = blob_sync.trace() {
        let hydrated_messages = base_messages.as_deref().unwrap_or_default();
        let hydrated_images = hydrated_messages
            .iter()
            .map(|message| match &message.content {
                MessageContent::Parts { parts } => parts
                    .iter()
                    .filter(|part| matches!(part, ContentPart::Image { .. }))
                    .count(),
                _ => 0,
            })
            .sum::<usize>();
        let history = request
            .action
            .as_ref()
            .and_then(|action| action.action.as_ref())
            .and_then(|action| match action {
                pb::conversation_action::Action::UserMessageAction(action) => {
                    action.conversation_history.as_ref()
                }
                _ => None,
            });
        let summary = serde_json::json!({
            "checkpoint_root_count": request.conversation_state.as_ref().map_or(0, |state| state.root_prompt_messages_json.len()),
            "checkpoint_turn_count": request.conversation_state.as_ref().map_or(0, |state| state.turns.len()),
            "conversation_history_message_count": history.map_or(0, |history| history.messages.len()),
            "hydrated_message_count": hydrated_messages.len(),
            "hydrated_image_count": hydrated_images,
            "selected_source": "root_prompt_messages_json",
        });
        let encoded = serde_json::to_vec(&summary)?;
        trace.artifact("history_projection", "byok_server", &encoded, summary);
    }
    let request_context = context::hydrate(request, context_sync).await?;
    let ActionProjection {
        mode: mode_number,
        mut turn_user,
        action_context,
        mut event_id,
        input_id,
        starts_turn,
        compacting,
        background_completion,
    } = action(request, request_id)?;
    let checkpoint_mode = if request.subagent_type_name.is_some() {
        Mode::Subagent
    } else {
        mode_from_proto(mode_number)?
    };
    let mut model = model::requested_model(request)?;
    if let Some(configured_model) = store.model(&model.model_id).await? {
        configured_model.configure(&mut model);
    }
    let dynamic = context::dynamic_mcp(request, &request_context)?;
    model::overrides(request)?; // Validate every typed selection before preparing tools.
    let mut checkpoint_prompt = compiler.prompt_spec(
        checkpoint_mode,
        &model,
        &dynamic
            .values()
            .map(|(_, definition)| definition.clone())
            .collect::<Vec<_>>(),
        request.suppress_subagent_progress_update_tool == Some(true),
    )?;
    let byok_models = store.models().await?;
    if checkpoint_prompt
        .tools
        .iter()
        .any(|tool| tool.name == "Task")
    {
        super::subagents::configure_tools(
            &mut checkpoint_prompt,
            request,
            &request_context,
            &byok_models,
        );
    }
    let prompt = if compacting {
        compiler.prompt_spec(Mode::Compaction, &model, &[], false)?
    } else {
        checkpoint_prompt.clone()
    };
    let proposed_base_checkpoint_id = match base_messages.as_mut() {
        Some(messages) if !messages.is_empty() => {
            validate_prompt_root(messages)?;
            messages.retain(|message| {
                !(message.role == Role::System && message.origin == Origin::Prompt)
            });
            store.import_checkpoint(&conversation_id, messages).await?
        }
        Some(_) | None => store.ensure_conversation(&conversation_id).await?,
    };
    let base_checkpoint_id = match input_id.as_deref() {
        Some(input_id) => {
            store
                .anchor_input(&conversation_id, input_id, proposed_base_checkpoint_id)
                .await?
        }
        None => proposed_base_checkpoint_id,
    };
    let mut projected_user_context = if input_id.is_some() && !compacting && !background_completion
    {
        break_messages::compile_request_context(
            "identity",
            &request_context,
            base_messages.as_deref().unwrap_or_default(),
        )?
    } else {
        None
    };
    if event_id.is_none() {
        if let (Some(input_id), Some(user)) = (input_id.as_deref(), turn_user.as_ref()) {
            event_id = Some(
                break_messages::user_event_id(
                    input_id,
                    checkpoint_mode,
                    user,
                    &request_context,
                    &action_context,
                    projected_user_context
                        .as_ref()
                        .map(|message| &message.content),
                    compiler,
                    blob_sync,
                )
                .await?,
            );
        }
    }
    let existing_runtime = match event_id.as_deref() {
        Some(event_id) => {
            store
                .message(&conversation_id, &format!("runtime:{event_id}"))
                .await?
        }
        _ => None,
    };
    let request_context_message = match event_id.as_deref() {
        Some(event_id) if !compacting && !background_completion => {
            let message_id = format!("request-context:{event_id}");
            match store.message(&conversation_id, &message_id).await? {
                Some(message) => Some(message),
                None if input_id.is_some() => projected_user_context.take().map(|mut message| {
                    message.message_id = message_id;
                    message
                }),
                None => break_messages::compile_request_context(
                    event_id,
                    &request_context,
                    base_messages.as_deref().unwrap_or_default(),
                )?,
            }
        }
        _ => None,
    };
    let mut initial_messages = if compacting {
        Vec::new()
    } else {
        match (turn_user.clone(), event_id) {
            (Some(mut user), Some(event_id)) if background_completion => {
                let (message, text) = match existing_runtime {
                    Some(message) => {
                        let text = runtime_message_text(&message)?;
                        (message, text)
                    }
                    None => {
                        break_messages::compile_background(
                            event_id,
                            &user,
                            &request_context,
                            &action_context,
                            blob_sync,
                        )
                        .await?
                    }
                };
                user.text = text;
                turn_user = Some(user);
                vec![message]
            }
            (Some(user), Some(event_id)) => {
                let runtime = match existing_runtime {
                    Some(message) => message,
                    None => {
                        break_messages::compile(
                            event_id,
                            checkpoint_mode,
                            &user,
                            &request_context,
                            &action_context,
                            compiler,
                            blob_sync,
                        )
                        .await?
                    }
                };
                request_context_message
                    .into_iter()
                    .chain(std::iter::once(runtime))
                    .collect()
            }
            (None, None) => Vec::new(),
            _ => {
                return Err(Error::Protocol(
                    "Cursor action has an incomplete runtime event".into(),
                ))
            }
        }
    };
    let (base_checkpoint_id, reused) = store
        .match_checkpoint_prefix(&conversation_id, base_checkpoint_id, &initial_messages)
        .await?;
    initial_messages.drain(..reused);
    let action = if compacting {
        RunAction::Compact
    } else if starts_turn {
        RunAction::Start
    } else {
        let pending_tool_round = match request
            .conversation_state
            .as_ref()
            .map(|state| state.pending_tool_calls.as_slice())
            .unwrap_or_default()
        {
            [] => None,
            [pending] => Some(messages::decode_pending(pending)?),
            pending => {
                return Err(Error::Protocol(format!(
                    "Cursor resume contains {} pending assistant messages",
                    pending.len()
                )))
            }
        };
        RunAction::Resume { pending_tool_round }
    };
    let mut exec = exec_context(request, &request_context, &conversation_id, &model.model_id);
    super::subagents::configure_model_aliases(&mut exec, &byok_models);
    Ok((
        PreparedRun {
            run_id,
            cursor_request_id: Some(request_id.into()),
            conversation_id,
            kind: RunKind::Root,
            model,
            prompt,
            initial_messages,
            action,
            base_checkpoint_id,
        },
        CursorRunContext {
            request_id: request_id.into(),
            mode: mode_number,
            turn_user,
            exec,
            dynamic_tools: dynamic
                .into_iter()
                .map(|(name, (wire, _))| (name, wire))
                .collect(),
            checkpoint_prompt,
            compacting,
            background_completion,
        },
    ))
}

fn runtime_message_text(message: &CanonicalMessage) -> Result<String> {
    let MessageContent::Parts { parts } = &message.content else {
        return Err(Error::Protocol(
            "stored runtime message does not contain parts".into(),
        ));
    };
    let Some(ContentPart::Text { text }) = parts.first() else {
        return Err(Error::Protocol(
            "stored runtime message does not start with text".into(),
        ));
    };
    Ok(text.clone())
}

fn validate_prompt_root(messages: &[CanonicalMessage]) -> Result<()> {
    let prompts = messages
        .iter()
        .filter(|message| message.role == Role::System && message.origin == Origin::Prompt)
        .collect::<Vec<_>>();
    let [prompt] = prompts.as_slice() else {
        return Err(Error::Protocol(format!(
            "Cursor history contains {} system prompt roots",
            prompts.len()
        )));
    };
    let MessageContent::Parts { parts } = &prompt.content else {
        return Err(Error::Protocol(
            "Cursor system prompt root is not textual content".into(),
        ));
    };
    let [ContentPart::Text { .. }] = parts.as_slice() else {
        return Err(Error::Protocol(
            "Cursor system prompt root is not one text part".into(),
        ));
    };
    Ok(())
}

fn execution_run_id(request_id: &str) -> RunId {
    let execution_id = Uuid::new_v4().simple().to_string();
    RunId::new(format!("{request_id}:{}", &execution_id[..8]))
}

fn action(request: &pb::AgentRunRequest, request_id: &str) -> Result<ActionProjection> {
    let conversation_mode = request
        .conversation_state
        .as_ref()
        .and_then(|state| state.mode);
    let mode = conversation_mode.unwrap_or(pb::AgentMode::Agent as i32);
    let Some(action) = request
        .action
        .as_ref()
        .and_then(|action| action.action.as_ref())
    else {
        return Ok(ActionProjection {
            mode,
            turn_user: None,
            action_context: String::new(),
            event_id: None,
            input_id: None,
            starts_turn: false,
            compacting: false,
            background_completion: false,
        });
    };
    match action {
        pb::conversation_action::Action::UserMessageAction(action) => {
            let mut user = action.user_message.clone().ok_or_else(|| {
                Error::Protocol("Cursor user message action has no UserMessage".into())
            })?;
            let mode = if user.mode == pb::AgentMode::Unspecified as i32 {
                conversation_mode.unwrap_or(user.mode)
            } else {
                user.mode
            };
            if user.message_id.is_empty() {
                if request
                    .subagent_type_name
                    .as_deref()
                    .is_none_or(str::is_empty)
                {
                    return Err(Error::Protocol(
                        "Cursor user message action has no message_id".into(),
                    ));
                }
                // Cursor 3.23.12 agent-host child actions omit messageId. The
                // original generation runId survives retry attempts; never use
                // prompt text or a random UUID as the input's identity.
                use sha2::{Digest, Sha256};
                let generation = request
                    .run_id
                    .as_deref()
                    .filter(|id| !id.is_empty())
                    .unwrap_or(request_id);
                if request.run_id.as_deref().is_none_or(str::is_empty) {
                    tracing::warn!(
                        request_id,
                        "subagent input identity falls back to attempt ID: generation ID absent"
                    );
                }
                let identity = serde_json::to_vec(&(
                    request.conversation_id.as_deref().unwrap_or(request_id),
                    generation,
                ))?;
                user.message_id = format!("subagent-input:{:x}", Sha256::digest(identity));
            }
            if user.text.trim() == "/summarize" {
                return Ok(ActionProjection {
                    mode,
                    turn_user: Some(user.clone()),
                    action_context: String::new(),
                    event_id: None,
                    input_id: None,
                    starts_turn: false,
                    compacting: true,
                    background_completion: false,
                });
            }
            let mut context = action
                .prepend_user_messages
                .iter()
                .map(|message| message.text.trim())
                .filter(|text| !text.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>();
            context.extend(
                user.subagent_system_reminder
                    .iter()
                    .filter(|text| !text.is_empty())
                    .cloned(),
            );
            let input_id = format!("cursor:user:{}", user.message_id);
            Ok(ActionProjection {
                mode,
                turn_user: Some(user.clone()),
                action_context: context.join("\n\n"),
                event_id: None,
                input_id: Some(input_id),
                starts_turn: true,
                compacting: false,
                background_completion: false,
            })
        }
        pb::conversation_action::Action::BackgroundTaskCompletionAction(action) => {
            let projection = insert_messages::project(action, mode)?;
            let event_id = projection.turn_user.message_id.clone();
            Ok(ActionProjection {
                mode,
                action_context: projection.context,
                event_id: Some(event_id),
                input_id: None,
                turn_user: Some(projection.turn_user),
                starts_turn: true,
                compacting: false,
                background_completion: true,
            })
        }
        pb::conversation_action::Action::ExecutePlanAction(action) => execute_plan(action),
        pb::conversation_action::Action::SummarizeAction(_) => Ok(ActionProjection {
            mode,
            turn_user: None,
            action_context: String::new(),
            event_id: None,
            input_id: None,
            starts_turn: false,
            compacting: true,
            background_completion: false,
        }),
        _ => Ok(ActionProjection {
            mode,
            turn_user: None,
            action_context: String::new(),
            event_id: None,
            input_id: None,
            starts_turn: false,
            compacting: false,
            background_completion: false,
        }),
    }
}

fn execute_plan(action: &pb::ExecutePlanAction) -> Result<ActionProjection> {
    let plan = action
        .plan_file_content
        .as_deref()
        .or_else(|| action.plan.as_ref().map(|plan| plan.plan.as_str()))
        .filter(|plan| !plan.trim().is_empty())
        .ok_or_else(|| Error::Protocol("ExecutePlan is missing plan content".into()))?;
    let source = action
        .plan_file_uri
        .as_deref()
        .or(action.plan_file_path.as_deref())
        .filter(|source| !source.is_empty());
    let action_context = match source {
        Some(source) => {
            format!("<approved_plan>\n<plan_file>{source}</plan_file>\n{plan}\n</approved_plan>")
        }
        None => format!("<approved_plan>\n{plan}\n</approved_plan>"),
    };
    let identity = BlobId::digest(
        format!(
            "{}\0{}\0{}\0{}\0{}",
            action.execution_mode,
            action.plan_id.as_deref().unwrap_or_default(),
            action.kickoff_message_id.as_deref().unwrap_or_default(),
            source.unwrap_or_default(),
            plan,
        )
        .as_bytes(),
    )
    .to_base64();
    let event_id = format!("execute-plan:{identity}");
    Ok(ActionProjection {
        mode: action.execution_mode,
        turn_user: Some(pb::UserMessage {
            text: "Execute the approved plan.".into(),
            message_id: event_id.clone(),
            mode: action.execution_mode,
            ..Default::default()
        }),
        action_context,
        event_id: Some(event_id),
        input_id: None,
        starts_turn: true,
        compacting: false,
        background_completion: false,
    })
}

pub(super) fn mode_from_proto(mode: i32) -> Result<Mode> {
    let mode = pb::AgentMode::try_from(mode)
        .map_err(|_| Error::Protocol(format!("unknown Cursor agent mode: {mode}")))?;
    match mode {
        pb::AgentMode::Agent => Ok(Mode::Agent),
        pb::AgentMode::Ask => Ok(Mode::Ask),
        pb::AgentMode::Plan => Ok(Mode::Plan),
        pb::AgentMode::Debug => Ok(Mode::Debug),
        pb::AgentMode::Multitask => Ok(Mode::Multitask),
        mode => Err(Error::Protocol(format!(
            "unsupported Cursor agent mode: {}",
            mode.as_str_name()
        ))),
    }
}

fn exec_context(
    request: &pb::AgentRunRequest,
    request_context: &pb::RequestContext,
    conversation_id: &ConversationId,
    model_id: &str,
) -> ExecContext {
    let inherited = pb::RequestedModel {
        model_id: model_id.into(),
        parameters: request
            .requested_model
            .as_ref()
            .map(|m| m.parameters.clone())
            .unwrap_or_default(),
        ..Default::default()
    };
    let mut subagent_models = request_context
        .custom_subagents
        .iter()
        .filter_map(|agent| {
            let id = agent.model.trim();
            if id.is_empty() || matches!(id, "inherit" | "default") {
                return None;
            }
            let model = request
                .selected_subagent_models
                .iter()
                .find(|model| model.model_id == id)
                .cloned()
                .map(SubagentModel::Model)
                .unwrap_or_else(|| {
                    SubagentModel::NamedModel(pb::RequestedModel {
                        model_id: id.into(),
                        ..Default::default()
                    })
                });
            Some((agent.name.clone(), model))
        })
        .collect::<std::collections::HashMap<_, _>>();
    subagent_models.extend(request.subagent_model_overrides.iter().filter_map(|value| {
        use pb::subagent_model_override::Selection;
        let selection = match value.selection.as_ref()? {
            Selection::Model(model) if model.model_id != "default" => {
                SubagentModel::Model(model.clone())
            }
            Selection::Model(_) | Selection::Inherit(true) => {
                SubagentModel::Model(inherited.clone())
            }
            Selection::Disabled(true) => SubagentModel::Disabled,
            _ => return None,
        };
        Some((value.subagent_type.clone(), selection))
    }));
    ExecContext {
        conversation_id: conversation_id.to_string(),
        root_conversation_id: request
            .conversation_group_id
            .clone()
            .unwrap_or_else(|| conversation_id.to_string()),
        default_subagent_model: model_id.into(),
        default_subagent_parameters: inherited.parameters,
        subagent_models,
        selected_subagent_models: request
            .selected_subagent_models
            .iter()
            .map(|model| (model.model_id.clone(), model.clone()))
            .collect(),
        byok_model_aliases: Default::default(),
        allow_subagents: request.subagent_type_name.is_none(),
        terminals_folder: request_context
            .env
            .as_ref()
            .map(|env| env.terminals_folder.clone())
            .unwrap_or_default(),
        admin_command_denylist: request_context.admin_command_denylist.clone(),
        mcp_routes: context::meta_mcp_routes(request_context),
    }
}

#[cfg(test)]
mod subagent_selection_tests {
    use super::*;
    #[test]
    fn agent_host_child_input_identity_survives_attempts_and_separates_generations() {
        let mut request = pb::AgentRunRequest {
            conversation_id: Some("child-a".into()),
            run_id: Some("generation-a".into()),
            subagent_type_name: Some("explore".into()),
            action: Some(pb::ConversationAction {
                action: Some(pb::conversation_action::Action::UserMessageAction(
                    pb::UserMessageAction {
                        user_message: Some(pb::UserMessage {
                            text: "synthetic input".into(),
                            mode: pb::AgentMode::Agent as i32,
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                )),
                ..Default::default()
            }),
            ..Default::default()
        };
        let first = action(&request, "attempt-a").unwrap();
        assert_eq!(
            first.input_id,
            action(&request, "attempt-b").unwrap().input_id
        );
        assert_eq!(
            first.input_id,
            Some(format!(
                "cursor:user:{}",
                first.turn_user.unwrap().message_id
            ))
        );
        request.run_id = Some("generation-b".into());
        assert_ne!(
            first.input_id,
            action(&request, "attempt-a").unwrap().input_id
        );
        request.run_id = Some("generation-a".into());
        request.conversation_id = Some("child-b".into());
        assert_ne!(
            first.input_id,
            action(&request, "attempt-a").unwrap().input_id
        );
        request.subagent_type_name = None;
        assert!(action(&request, "attempt-a").is_err());
        let Some(pb::conversation_action::Action::UserMessageAction(user)) =
            request.action.as_mut().unwrap().action.as_mut()
        else {
            unreachable!()
        };
        user.user_message.as_mut().unwrap().message_id = "explicit-message".into();
        assert_eq!(
            action(&request, "attempt-a").unwrap().input_id.as_deref(),
            Some("cursor:user:explicit-message")
        );
    }

    #[test]
    fn custom_agent_defaults_are_read_from_native_context() {
        let custom = pb::RequestContext {
            custom_subagents: vec![
                pb::CustomSubagent {
                    name: "reviewer".into(),
                    model: "byok-review".into(),
                    ..Default::default()
                },
                pb::CustomSubagent {
                    name: "writer".into(),
                    model: "inherit".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let request = pb::AgentRunRequest {
            selected_subagent_models: vec![pb::RequestedModel {
                model_id: "byok-review".into(),
                parameters: vec![pb::requested_model::ModelParameterValue {
                    id: "effort".into(),
                    value: "high".into(),
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let context = exec_context(
            &request,
            &custom,
            &ConversationId::new("parent"),
            "parent-model",
        );
        let Some(SubagentModel::Model(model)) = context.subagent_models.get("reviewer") else {
            panic!("custom model")
        };
        assert_eq!(model.model_id, "byok-review");
        assert_eq!(model.parameters[0].value, "high");
        assert!(!context.subagent_models.contains_key("writer"));
    }

    #[test]
    fn selections_remain_per_type_regardless_of_override_order() {
        use pb::subagent_model_override::Selection;
        let entries = vec![
            pb::SubagentModelOverride {
                subagent_type: "explore".into(),
                selection: Some(Selection::Disabled(true)),
            },
            pb::SubagentModelOverride {
                subagent_type: "generalPurpose".into(),
                selection: Some(Selection::Inherit(true)),
            },
            pb::SubagentModelOverride {
                subagent_type: "review".into(),
                selection: Some(Selection::Model(pb::RequestedModel {
                    model_id: "official".into(),
                    parameters: vec![pb::requested_model::ModelParameterValue {
                        id: "effort".into(),
                        value: "low".into(),
                    }],
                    ..Default::default()
                })),
            },
        ];
        for reverse in [false, true] {
            let mut request = pb::AgentRunRequest {
                requested_model: Some(pb::RequestedModel {
                    model_id: "parent".into(),
                    parameters: vec![pb::requested_model::ModelParameterValue {
                        id: "effort".into(),
                        value: "high".into(),
                    }],
                    ..Default::default()
                }),
                subagent_model_overrides: entries.clone(),
                ..Default::default()
            };
            if reverse {
                request.subagent_model_overrides.reverse();
            }
            let context = exec_context(
                &request,
                &pb::RequestContext::default(),
                &ConversationId::new("parent-conversation"),
                "parent",
            );
            assert!(context.allow_subagents);
            assert!(matches!(
                context.subagent_models.get("explore"),
                Some(SubagentModel::Disabled)
            ));
            let Some(SubagentModel::Model(inherited)) =
                context.subagent_models.get("generalPurpose")
            else {
                panic!("inherit")
            };
            assert_eq!(inherited.model_id, "parent");
            assert_eq!(inherited.parameters[0].value, "high");
            let Some(SubagentModel::Model(explicit)) = context.subagent_models.get("review") else {
                panic!("explicit")
            };
            assert_eq!(explicit.model_id, "official");
            assert_eq!(explicit.parameters[0].value, "low");
        }
    }
}
