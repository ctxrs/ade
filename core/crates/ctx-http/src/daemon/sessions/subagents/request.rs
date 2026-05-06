use std::collections::HashSet;

use crate::api::sessions::AgentInitItem;
use crate::settings as user_settings;
use ctx_core::ids::TaskId;

use super::errors::{api_error, internal_api_error, ApiResult, SubagentErrorKind};

const DEFAULT_MAX_SUBAGENTS_PER_CALL: usize = 10;
pub(super) const DEFAULT_MAX_ACTIVE_SUBAGENTS_PER_PARENT: usize = 12;
pub(super) const DEFAULT_MAX_SUBAGENT_DEPTH: usize = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SubagentWorktreeSelection {
    Inherit,
    New,
}

pub(super) fn resolve_max_subagents_per_call(settings: &user_settings::Settings) -> usize {
    let configured = settings
        .subagents
        .as_ref()
        .and_then(|s| s.max_per_call)
        .filter(|value| *value > 0);
    configured
        .map(|value| value as usize)
        .unwrap_or(DEFAULT_MAX_SUBAGENTS_PER_CALL)
}

pub(super) fn parse_subagent_worktree(
    value: Option<&str>,
) -> Result<SubagentWorktreeSelection, String> {
    let trimmed = value.map(|raw| raw.trim()).filter(|raw| !raw.is_empty());
    match trimmed {
        Some("inherit") => Ok(SubagentWorktreeSelection::Inherit),
        Some("new") => Ok(SubagentWorktreeSelection::New),
        Some(_) => Err("worktree must be 'inherit' or 'new'".to_string()),
        None => Err("worktree is required".to_string()),
    }
}

fn normalize_reasoning_effort(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(crate::api::sessions::normalize_effort_id)
        .filter(|value| !value.is_empty())
}

pub(super) fn build_subagent_request_json(agents: &[AgentInitItem]) -> serde_json::Value {
    let mut items = Vec::with_capacity(agents.len());
    for (idx, agent) in agents.iter().enumerate() {
        let prompt = agent.prompt.trim();
        let mut obj = serde_json::Map::new();
        obj.insert(
            "position".to_string(),
            serde_json::Value::Number(serde_json::Number::from(idx as u64)),
        );
        obj.insert(
            "prompt_length".to_string(),
            serde_json::Value::Number(serde_json::Number::from(prompt.chars().count() as u64)),
        );
        if let Some(label) = agent
            .label
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            obj.insert(
                "label".to_string(),
                serde_json::Value::String(label.to_string()),
            );
        }
        if let Some(harness) = agent
            .harness
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            obj.insert(
                "harness".to_string(),
                serde_json::Value::String(harness.to_string()),
            );
        }
        if let Some(model) = agent
            .model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            obj.insert(
                "model".to_string(),
                serde_json::Value::String(model.to_string()),
            );
        }
        if let Some(reasoning_effort) =
            normalize_reasoning_effort(agent.reasoning_effort.as_deref())
        {
            obj.insert(
                "reasoning_effort".to_string(),
                serde_json::Value::String(reasoning_effort),
            );
        }
        items.push(serde_json::Value::Object(obj));
    }

    serde_json::json!({
        "agents_total": agents.len(),
        "agents": items,
    })
}

pub(super) fn default_catalog_model_id(
    catalog: Option<&crate::api::sessions::ModelCatalog>,
) -> Option<&str> {
    catalog.and_then(crate::api::sessions::ModelCatalog::default_model_id)
}

pub(super) async fn validate_requested_labels(
    store: &ctx_store::Store,
    task_id: TaskId,
    agents: &[AgentInitItem],
) -> ApiResult<Vec<String>> {
    let mut labels = Vec::with_capacity(agents.len());
    let mut seen_labels = HashSet::new();
    for (idx, agent) in agents.iter().enumerate() {
        let label = agent
            .label
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                api_error(
                    SubagentErrorKind::BadRequest,
                    format!("agent {} label is required", idx + 1),
                )
            })?;
        if !seen_labels.insert(label.to_string()) {
            return Err(api_error(
                SubagentErrorKind::BadRequest,
                format!("duplicate subagent label '{label}'"),
            ));
        }
        labels.push(label.to_string());
    }

    for label in &labels {
        if store
            .subagent_label_exists(task_id, label)
            .await
            .map_err(internal_api_error)?
        {
            return Err(api_error(
                SubagentErrorKind::BadRequest,
                format!("subagent label '{label}' already exists for this task"),
            ));
        }
    }

    Ok(labels)
}

pub(super) fn collect_provider_ids(
    agents: &[AgentInitItem],
    parent_provider_id: &str,
) -> Result<HashSet<String>, String> {
    let mut provider_ids = HashSet::new();
    for agent in agents {
        let provider_id = agent
            .harness
            .as_deref()
            .unwrap_or(parent_provider_id)
            .trim();
        if provider_id.is_empty() {
            return Err("harness is required".to_string());
        }
        provider_ids.insert(provider_id.to_string());
    }
    Ok(provider_ids)
}
