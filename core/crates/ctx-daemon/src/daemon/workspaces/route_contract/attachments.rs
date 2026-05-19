use ctx_core::ids::WorkspaceId;
use ctx_core::models::{AttachmentMode, AttachmentUpdatePolicy, WorkspaceAttachmentKind};
use ctx_workspace_attachments::AttachmentConfig;
use serde::Deserialize;

use super::super::{WorkspaceRouteError, WorkspacesHandle};
use super::common::require_workspace_for_route;
use super::responses::WorkspaceAttachmentRouteResponse;

#[derive(Debug, Deserialize)]
pub struct SyncWorkspaceAttachmentsRouteRequest {
    #[serde(default)]
    refresh: Option<bool>,
}

impl SyncWorkspaceAttachmentsRouteRequest {
    pub(in crate::daemon::workspaces) fn refresh(&self) -> bool {
        self.refresh.unwrap_or(false)
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateWorkspaceAttachmentRouteRequest {
    pub(super) kind: WorkspaceAttachmentKind,
    pub(super) name: String,
    pub(super) source: String,
    #[serde(default)]
    pub(super) revision: Option<String>,
    #[serde(default)]
    pub(super) subpath: Option<String>,
    #[serde(default)]
    pub(super) mount_relpath: Option<String>,
    #[serde(default)]
    pub(super) mode: Option<AttachmentMode>,
    #[serde(default)]
    pub(super) update_policy: Option<AttachmentUpdatePolicy>,
}

impl CreateWorkspaceAttachmentRouteRequest {
    pub(in crate::daemon::workspaces) fn into_attachment_config(
        self,
    ) -> Result<AttachmentConfig, WorkspaceRouteError> {
        if self.name.trim().is_empty() || self.source.trim().is_empty() {
            return Err(WorkspaceRouteError::bad_request(
                "name and source are required",
            ));
        }
        Ok(AttachmentConfig {
            kind: self.kind,
            name: self.name,
            source: self.source,
            revision: self.revision,
            subpath: self.subpath,
            mount_relpath: self.mount_relpath,
            mode: self.mode,
            update_policy: self.update_policy,
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct DeleteWorkspaceAttachmentRouteRequest {
    pub(super) kind: WorkspaceAttachmentKind,
    pub(super) name: String,
}

impl DeleteWorkspaceAttachmentRouteRequest {
    pub(in crate::daemon::workspaces) fn into_parts(
        self,
    ) -> Result<(WorkspaceAttachmentKind, String), WorkspaceRouteError> {
        if self.name.trim().is_empty() {
            return Err(WorkspaceRouteError::bad_request("name is required"));
        }
        Ok((self.kind, self.name))
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
            .map_err(WorkspaceRouteError::from_workspace_store)?;
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
        let cfg = request.into_attachment_config()?;
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
        let (kind, name) = request.into_parts()?;
        let workspace = require_workspace_for_route(self, workspace_id).await?;
        let removed = self
            .delete_workspace_attachment(workspace_id, kind, &name)
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
