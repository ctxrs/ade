use super::{
    CreateWorkspaceAttachmentRouteRequest, DeleteWorkspaceAttachmentRouteRequest,
    SyncWorkspaceAttachmentsRouteRequest, WorkspaceAttachmentRouteResponse, WorkspaceRouteParams,
};
use crate::daemon::workspaces::{
    UpdateWorkspaceExecutionConfigRequest, UpdateWorkspaceMergeQueueConfigRequest,
    UpdateWorkspacePrimaryBranchRequest, UpdateWorktreeBootstrapConfigRequest,
    WorkspaceConfigUpdateResult, WorkspaceExecutionConfigSnapshot,
    WorkspaceMergeQueueConfigRouteResponse, WorkspacePrimaryBranchSnapshot, WorkspaceRouteError,
    WorkspaceWorktreeBootstrapConfigRouteResponse, WorkspacesHandle,
};

impl WorkspacesHandle {
    pub async fn list_workspace_attachments_for_route_params(
        &self,
        params: WorkspaceRouteParams,
    ) -> Result<Vec<WorkspaceAttachmentRouteResponse>, WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.list_workspace_attachments_for_route(workspace_id)
            .await
    }

    pub async fn sync_workspace_attachments_for_route_params(
        &self,
        params: WorkspaceRouteParams,
        request: SyncWorkspaceAttachmentsRouteRequest,
    ) -> Result<Vec<WorkspaceAttachmentRouteResponse>, WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.sync_workspace_attachments_for_route(workspace_id, request)
            .await
    }

    pub async fn create_and_sync_workspace_attachment_for_route_params(
        &self,
        params: WorkspaceRouteParams,
        request: CreateWorkspaceAttachmentRouteRequest,
    ) -> Result<Vec<WorkspaceAttachmentRouteResponse>, WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.create_and_sync_workspace_attachment_for_route(workspace_id, request)
            .await
    }

    pub async fn delete_and_sync_workspace_attachment_for_route_params(
        &self,
        params: WorkspaceRouteParams,
        request: DeleteWorkspaceAttachmentRouteRequest,
    ) -> Result<Vec<WorkspaceAttachmentRouteResponse>, WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.delete_and_sync_workspace_attachment_for_route(workspace_id, request)
            .await
    }

    pub async fn workspace_merge_queue_config_for_route_params(
        &self,
        params: WorkspaceRouteParams,
    ) -> Result<WorkspaceMergeQueueConfigRouteResponse, WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.workspace_merge_queue_config_for_route(workspace_id)
            .await
    }

    pub async fn update_workspace_merge_queue_config_for_route_params(
        &self,
        params: WorkspaceRouteParams,
        request: UpdateWorkspaceMergeQueueConfigRequest,
    ) -> Result<WorkspaceConfigUpdateResult, WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.update_workspace_merge_queue_config_for_route(workspace_id, request)
            .await
    }

    pub async fn workspace_primary_branch_for_route_params(
        &self,
        params: WorkspaceRouteParams,
    ) -> Result<WorkspacePrimaryBranchSnapshot, WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.workspace_primary_branch_for_request(workspace_id)
            .await
    }

    pub async fn update_workspace_primary_branch_for_route_params(
        &self,
        params: WorkspaceRouteParams,
        request: UpdateWorkspacePrimaryBranchRequest,
    ) -> Result<WorkspacePrimaryBranchSnapshot, WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.update_workspace_primary_branch_for_request(workspace_id, request)
            .await
    }

    pub async fn workspace_execution_config_for_route_params(
        &self,
        params: WorkspaceRouteParams,
    ) -> Result<WorkspaceExecutionConfigSnapshot, WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.workspace_execution_config_for_request(workspace_id)
            .await
    }

    pub async fn update_workspace_execution_config_for_route_params(
        &self,
        params: WorkspaceRouteParams,
        request: UpdateWorkspaceExecutionConfigRequest,
    ) -> Result<WorkspaceConfigUpdateResult, WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.update_workspace_execution_config_for_request(workspace_id, request)
            .await
    }

    pub async fn worktree_bootstrap_config_for_route_params(
        &self,
        params: WorkspaceRouteParams,
    ) -> Result<WorkspaceWorktreeBootstrapConfigRouteResponse, WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.worktree_bootstrap_config_for_route(workspace_id).await
    }

    pub async fn update_worktree_bootstrap_config_for_route_params(
        &self,
        params: WorkspaceRouteParams,
        request: UpdateWorktreeBootstrapConfigRequest,
    ) -> Result<WorkspaceConfigUpdateResult, WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.update_worktree_bootstrap_config_for_route(workspace_id, request)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctx_core::models::WorkspaceAttachmentKind;

    use crate::daemon::workspaces::WorkspaceRouteErrorKind;
    use crate::test_support::TestDaemon;

    #[tokio::test]
    async fn attachment_route_params_reject_invalid_workspace_id() {
        let temp = tempfile::tempdir().expect("tempdir");
        let daemon =
            TestDaemon::new_for_test(temp.path().to_path_buf(), "http://127.0.0.1:0".to_string())
                .await
                .expect("test daemon");
        let handle = daemon.handle().workspaces();
        let error = handle
            .create_and_sync_workspace_attachment_for_route_params(
                WorkspaceRouteParams::new("not-a-workspace"),
                CreateWorkspaceAttachmentRouteRequest {
                    kind: WorkspaceAttachmentKind::ReferenceRepo,
                    name: "ref".to_string(),
                    source: "/tmp/ref".to_string(),
                    revision: None,
                    subpath: None,
                    mount_relpath: None,
                    mode: None,
                    update_policy: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind(), WorkspaceRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "invalid workspace id");
    }

    #[tokio::test]
    async fn management_config_route_params_reject_invalid_workspace_id() {
        let temp = tempfile::tempdir().expect("tempdir");
        let daemon =
            TestDaemon::new_for_test(temp.path().to_path_buf(), "http://127.0.0.1:0".to_string())
                .await
                .expect("test daemon");
        let handle = daemon.handle().workspaces();
        let error = handle
            .workspace_merge_queue_config_for_route_params(WorkspaceRouteParams::new(
                "not-a-workspace",
            ))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), WorkspaceRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "invalid workspace id");
    }

    #[tokio::test]
    async fn worktree_bootstrap_route_params_reject_invalid_workspace_id() {
        let temp = tempfile::tempdir().expect("tempdir");
        let daemon =
            TestDaemon::new_for_test(temp.path().to_path_buf(), "http://127.0.0.1:0".to_string())
                .await
                .expect("test daemon");
        let handle = daemon.handle().workspaces();
        let error = handle
            .worktree_bootstrap_config_for_route_params(WorkspaceRouteParams::new(
                "not-a-workspace",
            ))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), WorkspaceRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "invalid workspace id");
    }
}
