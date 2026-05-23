use crate::daemon::workspaces::WorkspacesHandle;
use ctx_route_contracts::workspaces::{
    CreateWorkspaceAttachmentRouteRequest, DeleteWorkspaceAttachmentRouteRequest,
    SyncWorkspaceAttachmentsRouteRequest, UpdateWorkspaceExecutionConfigRequest,
    UpdateWorkspaceMergeQueueConfigRequest, UpdateWorkspacePrimaryBranchRequest,
    WorkspaceAttachmentRouteResponse, WorkspaceConfigUpdateResult,
    WorkspaceExecutionConfigRouteSnapshot, WorkspaceMergeQueueConfigRouteResponse,
    WorkspacePrimaryBranchSnapshot, WorkspaceRouteError, WorkspaceRouteParams,
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
    ) -> Result<WorkspaceExecutionConfigRouteSnapshot, WorkspaceRouteError> {
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
}
