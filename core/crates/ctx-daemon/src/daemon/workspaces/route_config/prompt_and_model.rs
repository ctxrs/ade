use ctx_core::ids::WorkspaceId;
use ctx_observability::logs;
use ctx_workspace_config as workspace_config;
use serde::{Deserialize, Serialize};

use crate::daemon::workspaces::{
    WorkspaceProviderModelPreference, WorkspaceProviderModelPreferenceError, WorkspaceRouteError,
};
use crate::daemon::WorkspaceStoreAccessError;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct WorkspaceProviderModelPreferenceRouteParams {
    pub(in crate::daemon::workspaces) workspace_id: String,
    pub(in crate::daemon::workspaces) provider_id: String,
}

impl WorkspaceProviderModelPreferenceRouteParams {
    pub fn new(workspace_id: impl Into<String>, provider_id: impl Into<String>) -> Self {
        Self {
            workspace_id: workspace_id.into(),
            provider_id: provider_id.into(),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct WorkspacePromptConfigRouteParams {
    pub(in crate::daemon::workspaces) workspace_id: String,
}

impl WorkspacePromptConfigRouteParams {
    pub fn new(workspace_id: impl Into<String>) -> Self {
        Self {
            workspace_id: workspace_id.into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
pub struct UpdateWorkspaceProviderModelPreferenceRouteRequest {
    #[serde(default)]
    pub(in crate::daemon::workspaces) preferred_model_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Eq, PartialEq)]
pub struct WorkspaceProviderModelPreferenceRouteResponse {
    provider_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    preferred_model_id: Option<String>,
}

impl From<WorkspaceProviderModelPreference> for WorkspaceProviderModelPreferenceRouteResponse {
    fn from(value: WorkspaceProviderModelPreference) -> Self {
        Self {
            provider_id: value.provider_id,
            preferred_model_id: value.preferred_model_id,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
pub struct UpdateAgentSystemPromptConfigRouteRequest {
    #[serde(default)]
    pub(in crate::daemon::workspaces) system_prompt_append: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
pub struct UpdateSubagentSystemPromptConfigRouteRequest {
    #[serde(default)]
    pub(in crate::daemon::workspaces) system_prompt_append: Option<String>,
}

#[derive(Debug, Clone, Serialize, Eq, PartialEq)]
pub struct AgentSystemPromptConfigRouteResponse {
    default_append: String,
    configured_append: Option<String>,
    effective_append: Option<String>,
    source: String,
}

#[derive(Debug, Clone, Serialize, Eq, PartialEq)]
pub struct SubagentSystemPromptConfigRouteResponse {
    default_append: String,
    configured_append: Option<String>,
    effective_append: Option<String>,
    source: String,
}

pub(in crate::daemon::workspaces) fn parse_workspace_route_id(
    value: &str,
) -> Result<WorkspaceId, WorkspaceRouteError> {
    uuid::Uuid::parse_str(value)
        .map(WorkspaceId)
        .map_err(|_| WorkspaceRouteError::bad_request("invalid workspace id"))
}

pub(in crate::daemon::workspaces) fn provider_model_preference_error(
    error: WorkspaceProviderModelPreferenceError,
) -> WorkspaceRouteError {
    match error {
        WorkspaceProviderModelPreferenceError::ProviderIdRequired => {
            WorkspaceRouteError::bad_request("provider_id is required")
        }
        WorkspaceProviderModelPreferenceError::ProviderNotFound { provider_id } => {
            WorkspaceRouteError::not_found(format!("provider not found: {provider_id}"))
        }
        WorkspaceProviderModelPreferenceError::WorkspaceNotFound => {
            WorkspaceRouteError::not_found("workspace not found")
        }
        WorkspaceProviderModelPreferenceError::StoreUnavailable(error) => {
            WorkspaceRouteError::internal(logs::redact_sensitive(&error.to_string()))
        }
        WorkspaceProviderModelPreferenceError::ExecutionSettings(error) => {
            WorkspaceRouteError::internal(format!(
                "failed to load workspace execution settings: {error:#}"
            ))
        }
    }
}

pub(in crate::daemon::workspaces) fn workspace_store_error(
    error: WorkspaceStoreAccessError,
) -> WorkspaceRouteError {
    match error {
        WorkspaceStoreAccessError::NotFound => {
            WorkspaceRouteError::not_found("workspace not found")
        }
        WorkspaceStoreAccessError::Unavailable(error) => {
            WorkspaceRouteError::internal(logs::redact_sensitive(&error.to_string()))
        }
    }
}

fn source_label(source: workspace_config::AgentSystemPromptAppendSource) -> String {
    match source {
        workspace_config::AgentSystemPromptAppendSource::Default => "default".to_string(),
        workspace_config::AgentSystemPromptAppendSource::Config => "config".to_string(),
        workspace_config::AgentSystemPromptAppendSource::Disabled => "disabled".to_string(),
    }
}

fn configured_append(value: &Option<String>) -> Option<String> {
    value.as_ref().map(|value| value.trim().to_string())
}

impl From<workspace_config::AgentSystemPromptAppendConfig>
    for AgentSystemPromptConfigRouteResponse
{
    fn from(cfg: workspace_config::AgentSystemPromptAppendConfig) -> Self {
        Self {
            default_append: cfg.default_append.clone(),
            configured_append: configured_append(&cfg.configured_append),
            effective_append: cfg.effective_append(),
            source: source_label(cfg.source()),
        }
    }
}

impl From<workspace_config::SubagentSystemPromptAppendConfig>
    for SubagentSystemPromptConfigRouteResponse
{
    fn from(cfg: workspace_config::SubagentSystemPromptAppendConfig) -> Self {
        Self {
            default_append: cfg.default_append.clone(),
            configured_append: configured_append(&cfg.configured_append),
            effective_append: cfg.effective_append(),
            source: source_label(cfg.source()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn route_requests_preserve_defaults_and_unknown_field_compatibility() {
        let provider: UpdateWorkspaceProviderModelPreferenceRouteRequest =
            serde_json::from_value(json!({
                "unknown": "ignored"
            }))
            .expect("provider preference request");
        assert_eq!(provider.preferred_model_id, None);

        let provider: UpdateWorkspaceProviderModelPreferenceRouteRequest =
            serde_json::from_value(json!({
                "preferred_model_id": " gpt-5.4/xhigh ",
                "unknown": "ignored"
            }))
            .expect("provider preference request");
        assert_eq!(
            provider.preferred_model_id.as_deref(),
            Some(" gpt-5.4/xhigh ")
        );

        let agent: UpdateAgentSystemPromptConfigRouteRequest = serde_json::from_value(json!({
            "unknown": "ignored"
        }))
        .expect("agent prompt request");
        assert_eq!(agent.system_prompt_append, None);

        let subagent: UpdateSubagentSystemPromptConfigRouteRequest =
            serde_json::from_value(json!({
                "unknown": "ignored"
            }))
            .expect("subagent prompt request");
        assert_eq!(subagent.system_prompt_append, None);
    }

    #[test]
    fn provider_preference_response_omits_absent_model() {
        let response =
            WorkspaceProviderModelPreferenceRouteResponse::from(WorkspaceProviderModelPreference {
                provider_id: "codex".to_string(),
                preferred_model_id: None,
            });

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            json!({
                "provider_id": "codex"
            })
        );
    }

    #[test]
    fn provider_preference_error_mapping_preserves_wire_messages() {
        let required = provider_model_preference_error(
            WorkspaceProviderModelPreferenceError::ProviderIdRequired,
        );
        assert_eq!(
            required.kind(),
            crate::daemon::workspaces::WorkspaceRouteErrorKind::BadRequest
        );
        assert_eq!(required.message(), "provider_id is required");

        let missing = provider_model_preference_error(
            WorkspaceProviderModelPreferenceError::ProviderNotFound {
                provider_id: "missing".to_string(),
            },
        );
        assert_eq!(
            missing.kind(),
            crate::daemon::workspaces::WorkspaceRouteErrorKind::NotFound
        );
        assert_eq!(missing.message(), "provider not found: missing");

        let workspace = provider_model_preference_error(
            WorkspaceProviderModelPreferenceError::WorkspaceNotFound,
        );
        assert_eq!(
            workspace.kind(),
            crate::daemon::workspaces::WorkspaceRouteErrorKind::NotFound
        );
        assert_eq!(workspace.message(), "workspace not found");

        let execution = provider_model_preference_error(
            WorkspaceProviderModelPreferenceError::ExecutionSettings(anyhow::anyhow!(
                "bad settings"
            )),
        );
        assert_eq!(
            execution.kind(),
            crate::daemon::workspaces::WorkspaceRouteErrorKind::Internal
        );
        assert!(execution
            .message()
            .starts_with("failed to load workspace execution settings:"));
    }

    #[test]
    fn parse_workspace_route_id_rejects_invalid_ids_with_wire_message() {
        let error = parse_workspace_route_id("not-a-workspace").unwrap_err();
        assert_eq!(
            error.kind(),
            crate::daemon::workspaces::WorkspaceRouteErrorKind::BadRequest
        );
        assert_eq!(error.message(), "invalid workspace id");
    }

    #[test]
    fn prompt_response_projection_preserves_source_and_trimming() {
        let response = AgentSystemPromptConfigRouteResponse::from(
            workspace_config::AgentSystemPromptAppendConfig {
                default_append: "Default".to_string(),
                configured_append: Some("  Configured  ".to_string()),
            },
        );

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            json!({
                "default_append": "Default",
                "configured_append": "Configured",
                "effective_append": "Configured",
                "source": "config"
            })
        );

        let response = SubagentSystemPromptConfigRouteResponse::from(
            workspace_config::SubagentSystemPromptAppendConfig {
                default_append: "Subagent default".to_string(),
                configured_append: None,
            },
        );

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            json!({
                "default_append": "Subagent default",
                "configured_append": null,
                "effective_append": "Subagent default",
                "source": "default"
            })
        );
    }

    #[test]
    fn provider_preference_update_requests_keep_raw_normalization_inputs() {
        let trimmed: UpdateWorkspaceProviderModelPreferenceRouteRequest =
            serde_json::from_value(json!({
                "preferred_model_id": " gpt-5.4/xhigh "
            }))
            .unwrap();
        assert_eq!(
            trimmed.preferred_model_id.as_deref(),
            Some(" gpt-5.4/xhigh ")
        );

        let blank: UpdateWorkspaceProviderModelPreferenceRouteRequest =
            serde_json::from_value(json!({
                "preferred_model_id": "   "
            }))
            .unwrap();
        assert_eq!(blank.preferred_model_id.as_deref(), Some("   "));
    }
}
