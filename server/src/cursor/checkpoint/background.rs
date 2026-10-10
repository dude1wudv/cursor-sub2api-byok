//! Cursor background completion state; never turns record-only events into model input.
use crate::cursor::protocol::proto::agent::v1 as pb;

pub(crate) fn record(
    state: &mut pb::ConversationStateStructure,
    action: &pb::BackgroundTaskCompletionAction,
) {
    for completion in &action.completions {
        if completion.kind != pb::BackgroundTaskKind::Subagent as i32
            || completion.reason != pb::BackgroundTaskCompletionReason::TaskFinished as i32
        {
            continue;
        }
        let Some(tool_id) = nonempty(completion.tool_call_id.as_deref()) else {
            continue;
        };
        let agent_id =
            nonempty(completion.subagent_id.as_deref()).unwrap_or(completion.task_id.trim());
        let persisted = state.subagent_states.get(agent_id);
        let run = state
            .subagent_runs_by_parent_tool_call_id
            .entry(tool_id.into())
            .or_default();
        if terminal(run.status)
            && completion
                .completed_at_ms
                .zip(run.completed_timestamp_ms)
                .is_some_and(|(new, old)| new < old)
        {
            continue;
        }
        run.parent_tool_call_id = tool_id.into();
        run.subagent_id = Some(agent_id.into());
        if let Some(persisted) = persisted {
            run.environment = persisted.environment;
            if let Some(path) = persisted
                .cloud_subagent
                .as_ref()
                .and_then(|cloud| nonempty(cloud.transcript_path.as_deref()))
            {
                run.transcript_path = Some(path.into());
            }
        }
        run.status = match pb::BackgroundTaskStatus::try_from(completion.status) {
            Ok(pb::BackgroundTaskStatus::Success) => pb::SubagentRunStatus::Success as i32,
            Ok(pb::BackgroundTaskStatus::Error) => pb::SubagentRunStatus::Error as i32,
            Ok(pb::BackgroundTaskStatus::Aborted) => pb::SubagentRunStatus::Aborted as i32,
            _ => run.status,
        };
        for (target, source) in [
            (&mut run.title, completion.title.as_str()),
            (&mut run.detail, completion.detail.as_deref().unwrap_or("")),
            (
                &mut run.output_path,
                completion.output_path.as_deref().unwrap_or(""),
            ),
        ] {
            if let Some(value) = nonempty(Some(source)) {
                *target = Some(value.into());
            }
        }
        run.completed_timestamp_ms = Some(
            completion
                .completed_at_ms
                .or(run.completed_timestamp_ms)
                .unwrap_or_else(crate::cursor::tools::runtime::now_ms),
        );
        run.completion_reason = Some(completion.reason);
    }
}

pub(crate) fn terminal(status: i32) -> bool {
    matches!(
        pb::SubagentRunStatus::try_from(status),
        Ok(pb::SubagentRunStatus::Success
            | pb::SubagentRunStatus::Error
            | pb::SubagentRunStatus::Aborted)
    )
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}
