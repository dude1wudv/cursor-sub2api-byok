//! Adapts Task's catalog to the native Cursor selections and local BYOK models.
use crate::{
    cursor::protocol::proto::agent::v1 as pb,
    model::{ModelConfig, PromptSpec},
};
use serde_json::json;

pub(super) fn configure_tools(
    prompt: &mut PromptSpec,
    request: &pb::AgentRunRequest,
    context: &pb::RequestContext,
    models: &[ModelConfig],
) {
    let Some(task) = prompt.tools.iter_mut().find(|tool| tool.name == "Task") else {
        return;
    };
    let disabled = request
        .subagent_model_overrides
        .iter()
        .filter_map(|entry| {
            matches!(
                entry.selection,
                Some(pb::subagent_model_override::Selection::Disabled(true))
            )
            .then_some(entry.subagent_type.as_str())
        })
        .collect::<Vec<_>>();
    if let Some(types) = task.parameters.pointer_mut("/properties/subagent_type") {
        if let Some(values) = types.get_mut("enum").and_then(|value| value.as_array_mut()) {
            for agent in &context.custom_subagents {
                let name = json!(agent.name);
                if !agent.name.is_empty() && !values.contains(&name) {
                    values.push(name);
                }
            }
            values.retain(|value| !disabled.contains(&value.as_str().unwrap_or_default()));
        }
        types["description"] = json!("Select a built-in or custom subagent type from this enum. Respect the user's native per-type model selection.");
    }
    // Only public model metadata goes into prompts; never serialize ModelConfig
    // or ModelDetails (which can carry URLs, headers or credentials).
    let byok = models
        .iter()
        .map(|model| {
            json!({
                "model": model.model_hash, "name": model.display_name,
                "provider_model": model.model_id,
            })
        })
        .collect::<Vec<_>>();
    let selected = request
        .selected_subagent_models
        .iter()
        .map(|model| {
            let name = request
                .selected_subagent_model_details
                .iter()
                .find(|details| details.model_id == model.model_id)
                .map(|details| details.display_name.as_str())
                .unwrap_or(&model.model_id);
            json!({"model": model.model_id, "name": name})
        })
        .collect::<Vec<_>>();
    task.description.push_str(&format!(
        "\n\nCurrent model catalog (data): {}\nUse the exact model value for BYOK; provider_model is a label, not a routing ID. Explicit Cursor official model IDs remain supported and use Cursor's normal account permissions. Use inherit unless the user requests a model; native per-type choices are applied by the controller. Disabled types: {}.",
        json!({"byok": byok, "cursor_selected": selected}), json!(disabled)
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cursor::prompting::{Mode, PromptAssets, PromptCompiler},
        model::ModelSpec,
    };

    #[test]
    fn custom_types_extend_native_schema_and_disabled_types_do_not_hide_other_tasks() {
        let compiler = PromptCompiler::new(PromptAssets::embedded().unwrap());
        let mut prompt = compiler
            .prompt_spec(Mode::Agent, &ModelSpec::new("parent"), &[], false)
            .unwrap();
        let request = pb::AgentRunRequest {
            subagent_model_overrides: vec![pb::SubagentModelOverride {
                subagent_type: "explore".into(),
                selection: Some(pb::subagent_model_override::Selection::Disabled(true)),
            }],
            selected_subagent_models: vec![pb::RequestedModel {
                model_id: "official-selected".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let context = pb::RequestContext {
            custom_subagents: vec![pb::CustomSubagent {
                name: "my-reviewer".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        configure_tools(&mut prompt, &request, &context, &[]);
        let task = prompt
            .tools
            .iter()
            .find(|tool| tool.name == "Task")
            .unwrap();
        let types = task
            .parameters
            .pointer("/properties/subagent_type/enum")
            .unwrap()
            .as_array()
            .unwrap();
        assert!(types.contains(&json!("my-reviewer")));
        assert!(types.contains(&json!("generalPurpose")));
        assert!(!types.contains(&json!("explore")));
        assert!(task.description.contains("official-selected"));
        assert!(!task
            .description
            .contains("Choose from the following list only"));
    }
}
