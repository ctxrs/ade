use std::collections::HashSet;

use crate::api::sessions::AgentInitItem;
use ctx_core::ids::TaskId;

use super::errors::{api_error, internal_api_error, ApiResult, SubagentErrorKind};

pub(super) fn default_catalog_model_id(
    catalog: Option<&ctx_session_tools::model_resolution::ModelCatalog>,
) -> Option<&str> {
    catalog.and_then(ctx_session_tools::model_resolution::ModelCatalog::default_model_id)
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
