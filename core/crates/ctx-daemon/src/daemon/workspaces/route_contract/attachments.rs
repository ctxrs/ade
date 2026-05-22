use ctx_core::ids::WorkspaceId;
use ctx_route_contracts::workspaces::{
    CreateWorkspaceAttachmentRouteRequest, DeleteWorkspaceAttachmentRouteRequest,
    SyncWorkspaceAttachmentsRouteRequest, WorkspaceAttachmentCreateRouteSpec,
    WorkspaceAttachmentRouteResponse,
};
use ctx_workspace_attachments::AttachmentConfig;

use super::super::{workspace_store_route_error, WorkspaceRouteError, WorkspacesHandle};
use super::common::require_workspace_for_route;

fn attachment_config_from_route_spec(spec: WorkspaceAttachmentCreateRouteSpec) -> AttachmentConfig {
    AttachmentConfig {
        kind: spec.kind,
        name: spec.name,
        source: spec.source,
        revision: spec.revision,
        subpath: spec.subpath,
        mount_relpath: spec.mount_relpath,
        mode: spec.mode,
        update_policy: spec.update_policy,
    }
}

impl WorkspacesHandle {
    pub async fn list_workspace_attachments_for_route(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<WorkspaceAttachmentRouteResponse>, WorkspaceRouteError> {
        let attachments = self
            .list_workspace_attachments(workspace_id)
            .await
            .map_err(workspace_store_route_error)?;
        Ok(attachments.into_iter().map(Into::into).collect())
    }

    pub async fn sync_workspace_attachments_for_route(
        &self,
        workspace_id: WorkspaceId,
        request: SyncWorkspaceAttachmentsRouteRequest,
    ) -> Result<Vec<WorkspaceAttachmentRouteResponse>, WorkspaceRouteError> {
        let workspace = require_workspace_for_route(self, workspace_id).await?;
        let attachments = self
            .sync_workspace_attachments(&workspace, request.refresh())
            .await
            .map_err(WorkspaceRouteError::internal)?;
        Ok(attachments.into_iter().map(Into::into).collect())
    }

    pub async fn create_and_sync_workspace_attachment_for_route(
        &self,
        workspace_id: WorkspaceId,
        request: CreateWorkspaceAttachmentRouteRequest,
    ) -> Result<Vec<WorkspaceAttachmentRouteResponse>, WorkspaceRouteError> {
        let cfg = attachment_config_from_route_spec(request.into_spec()?);
        let workspace = require_workspace_for_route(self, workspace_id).await?;
        self.upsert_workspace_attachment(workspace_id, cfg)
            .await
            .map_err(WorkspaceRouteError::bad_request)?;
        let attachments = self
            .sync_workspace_attachments(&workspace, true)
            .await
            .map_err(WorkspaceRouteError::internal)?;
        Ok(attachments.into_iter().map(Into::into).collect())
    }

    pub async fn delete_and_sync_workspace_attachment_for_route(
        &self,
        workspace_id: WorkspaceId,
        request: DeleteWorkspaceAttachmentRouteRequest,
    ) -> Result<Vec<WorkspaceAttachmentRouteResponse>, WorkspaceRouteError> {
        let spec = request.into_spec()?;
        let workspace = require_workspace_for_route(self, workspace_id).await?;
        let removed = self
            .delete_workspace_attachment(workspace_id, spec.kind, &spec.name)
            .await
            .map_err(WorkspaceRouteError::bad_request)?;
        if !removed {
            return Err(WorkspaceRouteError::not_found("attachment not found"));
        }
        let attachments = self
            .sync_workspace_attachments(&workspace, false)
            .await
            .map_err(WorkspaceRouteError::internal)?;
        Ok(attachments.into_iter().map(Into::into).collect())
    }
}
