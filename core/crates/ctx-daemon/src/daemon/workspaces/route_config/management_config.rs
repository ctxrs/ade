use ctx_workspace_config as workspace_config;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct UpdateWorkspaceMergeQueueConfigRequest {
    enabled: bool,
    #[serde(default)]
    target_branch: Option<String>,
    #[serde(default)]
    verify_command: Option<String>,
    #[serde(default)]
    push_on_success: Option<bool>,
    #[serde(default)]
    push_remote: Option<String>,
    #[serde(default)]
    push_branch: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct WorkspaceMergeQueueConfigRouteResponse {
    pub enabled: bool,
    pub target_branch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify_command: Option<String>,
    pub push_on_success: bool,
    pub push_remote: String,
    pub push_branch: String,
}

impl From<workspace_config::MergeQueueConfig> for WorkspaceMergeQueueConfigRouteResponse {
    fn from(cfg: workspace_config::MergeQueueConfig) -> Self {
        Self {
            enabled: cfg.enabled,
            target_branch: cfg.target_branch,
            verify_command: cfg.verify_commands.into_iter().next(),
            push_on_success: cfg.push_on_success,
            push_remote: cfg.push_remote,
            push_branch: cfg.push_branch,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateWorktreeBootstrapConfigRequest {
    #[serde(default)]
    setup_command: Option<String>,
    #[serde(default)]
    timeout_sec: Option<u64>,
    #[serde(default)]
    wait_for_completion: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct WorkspaceWorktreeBootstrapConfigRouteResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setup_command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_sec: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait_for_completion: Option<bool>,
}

impl From<Option<workspace_config::WorktreeBootstrapConfig>>
    for WorkspaceWorktreeBootstrapConfigRouteResponse
{
    fn from(cfg: Option<workspace_config::WorktreeBootstrapConfig>) -> Self {
        Self {
            setup_command: cfg.as_ref().and_then(|value| value.setup_command.clone()),
            timeout_sec: cfg.as_ref().and_then(|value| value.timeout_sec),
            wait_for_completion: cfg.as_ref().and_then(|value| value.wait_for_completion),
        }
    }
}

impl UpdateWorkspaceMergeQueueConfigRequest {
    pub(in crate::daemon::workspaces) fn into_merge_queue_config_update(
        self,
    ) -> workspace_config::MergeQueueConfigUpdate {
        let verify_commands = self
            .verify_command
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(|value| vec![value])
            .unwrap_or_default();

        workspace_config::MergeQueueConfigUpdate {
            enabled: self.enabled,
            target_branch: self.target_branch,
            verify_commands,
            push_on_success: self.push_on_success,
            push_remote: self.push_remote,
            push_branch: self.push_branch,
            canonical_sync: Some(workspace_config::MergeQueueCanonicalSync::CleanOnly),
        }
    }
}

impl UpdateWorktreeBootstrapConfigRequest {
    pub(in crate::daemon::workspaces) fn into_worktree_bootstrap_config_update(
        self,
    ) -> workspace_config::WorktreeBootstrapConfigUpdate {
        workspace_config::WorktreeBootstrapConfigUpdate {
            setup_command: self
                .setup_command
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            timeout_sec: self.timeout_sec,
            wait_for_completion: self.wait_for_completion,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_queue_route_response_preserves_default_wire_shape() {
        let response = WorkspaceMergeQueueConfigRouteResponse::from(
            workspace_config::MergeQueueConfig::new_default(),
        );

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            serde_json::json!({
                "enabled": false,
                "target_branch": "main",
                "push_on_success": false,
                "push_remote": "origin",
                "push_branch": "main"
            })
        );
    }

    #[test]
    fn merge_queue_route_response_projects_first_verify_command() {
        let mut config = workspace_config::MergeQueueConfig::new_default();
        config.verify_commands = vec!["pnpm test".to_string(), "cargo test".to_string()];

        let response = WorkspaceMergeQueueConfigRouteResponse::from(config);

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            serde_json::json!({
                "enabled": false,
                "target_branch": "main",
                "verify_command": "pnpm test",
                "push_on_success": false,
                "push_remote": "origin",
                "push_branch": "main"
            })
        );
    }

    #[test]
    fn merge_queue_update_request_trims_blank_verify_command() {
        let update = UpdateWorkspaceMergeQueueConfigRequest {
            enabled: true,
            target_branch: Some(" main ".to_string()),
            verify_command: Some("   ".to_string()),
            push_on_success: Some(true),
            push_remote: Some(" origin ".to_string()),
            push_branch: Some(" dev ".to_string()),
        }
        .into_merge_queue_config_update()
        .normalized();

        assert!(update.verify_commands.is_empty());
        assert_eq!(update.target_branch.as_deref(), Some("main"));
        assert_eq!(update.push_remote.as_deref(), Some("origin"));
        assert_eq!(update.push_branch.as_deref(), Some("dev"));
    }

    #[test]
    fn worktree_bootstrap_route_response_omits_empty_config() {
        let response = WorkspaceWorktreeBootstrapConfigRouteResponse::from(None);

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            serde_json::json!({})
        );
    }

    #[test]
    fn worktree_bootstrap_update_request_trims_blank_setup_command() {
        let update = UpdateWorktreeBootstrapConfigRequest {
            setup_command: Some("   ".to_string()),
            timeout_sec: Some(30),
            wait_for_completion: Some(true),
        }
        .into_worktree_bootstrap_config_update();

        assert_eq!(update.setup_command, None);
        assert_eq!(update.timeout_sec, Some(30));
        assert_eq!(update.wait_for_completion, Some(true));
    }
}
