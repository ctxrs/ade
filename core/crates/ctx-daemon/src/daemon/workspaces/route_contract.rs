use chrono::{DateTime, Utc};
use ctx_core::ids::{WorkspaceAttachmentId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    AttachmentMode, AttachmentUpdatePolicy, VcsKind, Workspace, WorkspaceActiveHeadBatch,
    WorkspaceActiveSnapshot, WorkspaceAttachment, WorkspaceAttachmentKind,
    WorkspaceAttachmentStatus, Worktree, WorktreeBootstrapStatus,
};
use ctx_observability::logs;
use ctx_sandbox_contract::{ContainerMountMode, ContainerNetworkMode};
use ctx_settings_service::EffectiveExecutionSettingsError;
use ctx_workspace_attachments::AttachmentConfig;
use ctx_workspace_container::WorkspaceContainerStatus;
use serde::{Deserialize, Serialize, Serializer};

use crate::daemon::{RouteFileDownloadError, TextRouteDownload};

use super::{
    FileCompletionsError, FileCompletionsErrorKind, WorkspaceDeleteError,
    WorkspaceHarnessContainerError, WorkspaceHydrationError, WorkspaceRouteError, WorkspacesHandle,
};

mod management_route_params;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct WorkspaceRouteParams {
    workspace_id: String,
}

impl WorkspaceRouteParams {
    pub fn new(workspace_id: impl Into<String>) -> Self {
        Self {
            workspace_id: workspace_id.into(),
        }
    }

    fn parse_workspace_id(&self) -> Result<WorkspaceId, WorkspaceRouteError> {
        uuid::Uuid::parse_str(&self.workspace_id)
            .map(WorkspaceId)
            .map_err(|_| WorkspaceRouteError::bad_request("invalid workspace id"))
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct WorktreeRouteParams {
    worktree_id: String,
}

impl WorktreeRouteParams {
    pub fn new(worktree_id: impl Into<String>) -> Self {
        Self {
            worktree_id: worktree_id.into(),
        }
    }

    fn parse_worktree_id(&self) -> Result<WorktreeId, WorkspaceRouteError> {
        uuid::Uuid::parse_str(&self.worktree_id)
            .map(WorktreeId)
            .map_err(|_| WorkspaceRouteError::bad_request("invalid worktree id"))
    }
}

#[derive(Debug, Clone, Deserialize, Default, Eq, PartialEq)]
pub struct WorkspaceFileCompletionsRouteQuery {
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceRouteResponse {
    pub id: WorkspaceId,
    pub name: String,
    pub root_path: String,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vcs_kind: Option<VcsKind>,
}

impl From<Workspace> for WorkspaceRouteResponse {
    fn from(workspace: Workspace) -> Self {
        Self {
            id: workspace.id,
            name: workspace.name,
            root_path: workspace.root_path,
            created_at: workspace.created_at,
            vcs_kind: workspace.vcs_kind,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct WorktreeRouteResponse {
    pub id: WorktreeId,
    pub workspace_id: WorkspaceId,
    pub root_path: String,
    pub base_commit_sha: String,
    pub git_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vcs_kind: Option<VcsKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vcs_ref: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_status: Option<WorktreeBootstrapStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_finished_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_exit_code: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_timeout_sec: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_log_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_log_truncated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_script_path: Option<String>,
}

impl From<Worktree> for WorktreeRouteResponse {
    fn from(worktree: Worktree) -> Self {
        Self {
            id: worktree.id,
            workspace_id: worktree.workspace_id,
            root_path: worktree.root_path,
            base_commit_sha: worktree.base_commit_sha,
            git_branch: worktree.git_branch,
            vcs_kind: worktree.vcs_kind,
            base_revision: worktree.base_revision,
            vcs_ref: worktree.vcs_ref,
            created_at: worktree.created_at,
            bootstrap_status: worktree.bootstrap_status,
            bootstrap_started_at: worktree.bootstrap_started_at,
            bootstrap_finished_at: worktree.bootstrap_finished_at,
            bootstrap_exit_code: worktree.bootstrap_exit_code,
            bootstrap_timeout_sec: worktree.bootstrap_timeout_sec,
            bootstrap_error: worktree.bootstrap_error,
            bootstrap_log_path: worktree.bootstrap_log_path,
            bootstrap_log_truncated: worktree.bootstrap_log_truncated,
            bootstrap_command: worktree.bootstrap_command,
            bootstrap_script_path: worktree.bootstrap_script_path,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceAttachmentRouteResponse {
    pub id: WorkspaceAttachmentId,
    pub workspace_id: WorkspaceId,
    pub kind: WorkspaceAttachmentKind,
    pub name: String,
    pub source: String,
    pub revision: Option<String>,
    pub subpath: Option<String>,
    pub mount_relpath: String,
    pub mode: AttachmentMode,
    pub update_policy: AttachmentUpdatePolicy,
    pub status: WorkspaceAttachmentStatus,
    pub last_sync_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<WorkspaceAttachment> for WorkspaceAttachmentRouteResponse {
    fn from(attachment: WorkspaceAttachment) -> Self {
        Self {
            id: attachment.id,
            workspace_id: attachment.workspace_id,
            kind: attachment.kind,
            name: attachment.name,
            source: attachment.source,
            revision: attachment.revision,
            subpath: attachment.subpath,
            mount_relpath: attachment.mount_relpath,
            mode: attachment.mode,
            update_policy: attachment.update_policy,
            status: attachment.status,
            last_sync_at: attachment.last_sync_at,
            error_message: attachment.error_message,
            created_at: attachment.created_at,
            updated_at: attachment.updated_at,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkspaceActiveSnapshotRouteResponse {
    value: WorkspaceActiveSnapshot,
}

impl From<WorkspaceActiveSnapshot> for WorkspaceActiveSnapshotRouteResponse {
    fn from(value: WorkspaceActiveSnapshot) -> Self {
        Self { value }
    }
}

impl Serialize for WorkspaceActiveSnapshotRouteResponse {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.value.serialize(serializer)
    }
}

#[derive(Debug, Clone)]
pub struct WorkspaceActiveHeadBatchRouteResponse {
    value: WorkspaceActiveHeadBatch,
}

impl From<WorkspaceActiveHeadBatch> for WorkspaceActiveHeadBatchRouteResponse {
    fn from(value: WorkspaceActiveHeadBatch) -> Self {
        Self { value }
    }
}

impl Serialize for WorkspaceActiveHeadBatchRouteResponse {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.value.serialize(serializer)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceHarnessContainerStatusRouteResponse {
    pub name: String,
    pub running: bool,
    pub known: bool,
    pub mount_mode: Option<ContainerMountMode>,
    pub network_mode: Option<ContainerNetworkMode>,
    pub allowlist: Vec<String>,
    pub egress_guard: Option<bool>,
}

impl From<WorkspaceContainerStatus> for WorkspaceHarnessContainerStatusRouteResponse {
    fn from(status: WorkspaceContainerStatus) -> Self {
        Self {
            name: status.name,
            running: status.running,
            known: status.known,
            mount_mode: status.mount_mode,
            network_mode: status.network_mode,
            allowlist: status.allowlist,
            egress_guard: status.egress_guard,
        }
    }
}

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
    kind: WorkspaceAttachmentKind,
    name: String,
    source: String,
    #[serde(default)]
    revision: Option<String>,
    #[serde(default)]
    subpath: Option<String>,
    #[serde(default)]
    mount_relpath: Option<String>,
    #[serde(default)]
    mode: Option<AttachmentMode>,
    #[serde(default)]
    update_policy: Option<AttachmentUpdatePolicy>,
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
    kind: WorkspaceAttachmentKind,
    name: String,
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

fn workspace_hydration_route_error(error: WorkspaceHydrationError) -> WorkspaceRouteError {
    match error {
        WorkspaceHydrationError::NotFound => WorkspaceRouteError::not_found("workspace not found"),
        WorkspaceHydrationError::Load(error) => {
            WorkspaceRouteError::internal(logs::redact_sensitive(&error.to_string()))
        }
    }
}

fn workspace_delete_route_error(error: WorkspaceDeleteError) -> WorkspaceRouteError {
    match error {
        WorkspaceDeleteError::NotFound => WorkspaceRouteError::not_found("workspace not found"),
        WorkspaceDeleteError::Internal => {
            WorkspaceRouteError::internal("failed to delete workspace")
        }
    }
}

fn route_file_download_error(error: RouteFileDownloadError) -> WorkspaceRouteError {
    match error {
        RouteFileDownloadError::NotFound => WorkspaceRouteError::not_found("file not found"),
        RouteFileDownloadError::Internal => {
            WorkspaceRouteError::internal("failed to read route file")
        }
    }
}

fn workspace_harness_container_status_error(
    error: WorkspaceHarnessContainerError,
) -> WorkspaceRouteError {
    match error {
        WorkspaceHarnessContainerError::NotFound => {
            WorkspaceRouteError::not_found("workspace not found")
        }
        WorkspaceHarnessContainerError::Internal(_)
        | WorkspaceHarnessContainerError::ExecutionSettings(_)
        | WorkspaceHarnessContainerError::Ensure(_) => {
            WorkspaceRouteError::internal("workspace harness container request failed")
        }
    }
}

fn effective_execution_settings_route_error(
    error: EffectiveExecutionSettingsError,
) -> WorkspaceRouteError {
    match error {
        EffectiveExecutionSettingsError::InvalidWorkspaceOverride(error) => {
            let message = logs::redact_sensitive(&error.to_string());
            if ctx_settings_service::is_execution_policy_denial(&error) {
                WorkspaceRouteError::forbidden(message)
            } else {
                WorkspaceRouteError::bad_request(message)
            }
        }
        EffectiveExecutionSettingsError::Internal(error) => {
            WorkspaceRouteError::internal(logs::redact_sensitive(&error.to_string()))
        }
    }
}

fn workspace_harness_container_ensure_error(
    error: WorkspaceHarnessContainerError,
) -> WorkspaceRouteError {
    match error {
        WorkspaceHarnessContainerError::NotFound => {
            WorkspaceRouteError::not_found("workspace not found")
        }
        WorkspaceHarnessContainerError::Internal(error) => {
            WorkspaceRouteError::internal(logs::redact_sensitive(&error.to_string()))
        }
        WorkspaceHarnessContainerError::ExecutionSettings(error) => {
            effective_execution_settings_route_error(error)
        }
        WorkspaceHarnessContainerError::Ensure(error) => {
            WorkspaceRouteError::bad_request(logs::redact_sensitive(&error.to_string()))
        }
    }
}

fn file_completions_route_error(error: FileCompletionsError) -> WorkspaceRouteError {
    match error.kind() {
        FileCompletionsErrorKind::NotFound => WorkspaceRouteError::not_found(error.message()),
        FileCompletionsErrorKind::Forbidden => WorkspaceRouteError::forbidden(error.message()),
        FileCompletionsErrorKind::InsufficientStorage => {
            WorkspaceRouteError::insufficient_storage(error.message())
        }
        FileCompletionsErrorKind::Internal => {
            tracing::warn!(error = error.message(), "file completions request failed");
            WorkspaceRouteError::internal(error.message())
        }
    }
}

impl WorkspacesHandle {
    pub async fn list_workspaces_for_route(
        &self,
    ) -> Result<Vec<WorkspaceRouteResponse>, WorkspaceRouteError> {
        let workspaces = self
            .list_workspaces()
            .await
            .map_err(WorkspaceRouteError::internal)?;
        Ok(workspaces.into_iter().map(Into::into).collect())
    }

    pub async fn get_workspace_for_route_params(
        &self,
        params: WorkspaceRouteParams,
    ) -> Result<WorkspaceRouteResponse, WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.get_workspace_for_route(workspace_id)
            .await?
            .ok_or_else(|| WorkspaceRouteError::not_found("workspace not found"))
    }

    pub async fn get_workspace_for_route(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<WorkspaceRouteResponse>, WorkspaceRouteError> {
        let workspace = self
            .get_workspace(workspace_id)
            .await
            .map_err(WorkspaceRouteError::internal)?;
        if workspace.is_some() {
            self.record_workspace_opened().await;
        }
        Ok(workspace.map(Into::into))
    }

    pub async fn delete_workspace_for_route(
        &self,
        params: WorkspaceRouteParams,
    ) -> Result<(), WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.delete_workspace(workspace_id)
            .await
            .map_err(workspace_delete_route_error)
    }

    pub async fn workspace_active_snapshot_for_route(
        &self,
        params: WorkspaceRouteParams,
    ) -> Result<WorkspaceActiveSnapshotRouteResponse, WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.load_workspace_active_snapshot_for_route(workspace_id)
            .await
            .map_err(workspace_hydration_route_error)
    }

    pub async fn load_workspace_active_snapshot_for_route(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspaceActiveSnapshotRouteResponse, super::WorkspaceHydrationError> {
        self.load_workspace_active_snapshot(workspace_id)
            .await
            .map(Into::into)
    }

    pub async fn workspace_active_heads_for_route(
        &self,
        params: WorkspaceRouteParams,
    ) -> Result<WorkspaceActiveHeadBatchRouteResponse, WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.load_workspace_active_heads_for_route(workspace_id)
            .await
            .map_err(workspace_hydration_route_error)
    }

    pub async fn load_workspace_active_heads_for_route(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspaceActiveHeadBatchRouteResponse, super::WorkspaceHydrationError> {
        self.load_workspace_active_heads(workspace_id)
            .await
            .map(Into::into)
    }

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
        let workspace = self.require_workspace_for_route(workspace_id).await?;
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
        let workspace = self.require_workspace_for_route(workspace_id).await?;
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
        let workspace = self.require_workspace_for_route(workspace_id).await?;
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

    pub async fn workspace_harness_container_status_for_route(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<
        Option<WorkspaceHarnessContainerStatusRouteResponse>,
        super::WorkspaceHarnessContainerError,
    > {
        self.workspace_harness_container_status(workspace_id)
            .await
            .map(|status| status.map(Into::into))
    }

    pub async fn workspace_harness_container_status_for_route_params(
        &self,
        params: WorkspaceRouteParams,
    ) -> Result<Option<WorkspaceHarnessContainerStatusRouteResponse>, WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.workspace_harness_container_status_for_route(workspace_id)
            .await
            .map_err(workspace_harness_container_status_error)
    }

    pub async fn stop_workspace_harness_container_for_route(
        &self,
        params: WorkspaceRouteParams,
    ) -> Result<(), WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.stop_workspace_harness_container(workspace_id)
            .await
            .map_err(workspace_harness_container_status_error)
    }

    pub async fn ensure_workspace_harness_container_for_route(
        &self,
        params: WorkspaceRouteParams,
    ) -> Result<(), WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.ensure_workspace_harness_container(workspace_id)
            .await
            .map_err(workspace_harness_container_ensure_error)
    }

    pub async fn get_worktree_for_route_params(
        &self,
        params: WorktreeRouteParams,
    ) -> Result<WorktreeRouteResponse, WorkspaceRouteError> {
        let worktree_id = params.parse_worktree_id()?;
        self.get_worktree_for_route(worktree_id)
            .await?
            .ok_or_else(|| WorkspaceRouteError::not_found("worktree not found"))
    }

    pub async fn get_worktree_for_route(
        &self,
        worktree_id: WorktreeId,
    ) -> Result<Option<WorktreeRouteResponse>, WorkspaceRouteError> {
        self.get_worktree_with_live_root(worktree_id)
            .await
            .map(|worktree| worktree.map(Into::into))
            .map_err(WorkspaceRouteError::internal)
    }

    pub async fn download_worktree_bootstrap_logs_for_route_params(
        &self,
        params: WorktreeRouteParams,
    ) -> Result<TextRouteDownload, WorkspaceRouteError> {
        let worktree_id = params.parse_worktree_id()?;
        self.download_worktree_bootstrap_logs_for_route(worktree_id)
            .await
            .map_err(route_file_download_error)
    }

    pub async fn workspace_file_completions_for_route(
        &self,
        params: WorkspaceRouteParams,
        query: WorkspaceFileCompletionsRouteQuery,
    ) -> Result<Vec<String>, WorkspaceRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.complete_files_for_workspace(workspace_id, query.query, query.limit)
            .await
            .map_err(file_completions_route_error)
    }

    async fn require_workspace_for_route(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Workspace, WorkspaceRouteError> {
        self.get_workspace(workspace_id)
            .await
            .map_err(WorkspaceRouteError::internal)?
            .ok_or_else(|| WorkspaceRouteError::not_found("workspace not found"))
    }
}

#[cfg(test)]
mod tests {
    use super::super::WorkspaceRouteErrorKind;
    use super::*;

    fn assert_same_json<T, U>(left: T, right: U)
    where
        T: Serialize,
        U: Serialize,
    {
        assert_eq!(
            serde_json::to_value(left).unwrap(),
            serde_json::to_value(right).unwrap()
        );
    }

    #[test]
    fn workspace_route_response_matches_workspace_wire_shape() {
        let workspace = Workspace {
            id: WorkspaceId::new(),
            name: "workspace".to_string(),
            root_path: "/tmp/workspace".to_string(),
            created_at: Utc::now(),
            vcs_kind: Some(VcsKind::Git),
        };
        assert_same_json(WorkspaceRouteResponse::from(workspace.clone()), workspace);
    }

    #[test]
    fn worktree_route_response_matches_worktree_wire_shape() {
        let now = Utc::now();
        let worktree = Worktree {
            id: WorktreeId::new(),
            workspace_id: WorkspaceId::new(),
            root_path: "/tmp/workspace/wt".to_string(),
            base_commit_sha: "abc123".to_string(),
            git_branch: Some("feature".to_string()),
            vcs_kind: Some(VcsKind::Git),
            base_revision: Some("rev-a".to_string()),
            vcs_ref: Some("main".to_string()),
            created_at: now,
            bootstrap_status: Some(WorktreeBootstrapStatus::Success),
            bootstrap_started_at: Some(now),
            bootstrap_finished_at: Some(now),
            bootstrap_exit_code: Some(0),
            bootstrap_timeout_sec: Some(60),
            bootstrap_error: Some("none".to_string()),
            bootstrap_log_path: Some("/tmp/bootstrap.log".to_string()),
            bootstrap_log_truncated: Some(false),
            bootstrap_command: Some("true".to_string()),
            bootstrap_script_path: Some("/tmp/bootstrap.sh".to_string()),
        };
        assert_same_json(WorktreeRouteResponse::from(worktree.clone()), worktree);
    }

    #[test]
    fn workspace_attachment_route_response_matches_attachment_wire_shape() {
        let now = Utc::now();
        let attachment = WorkspaceAttachment {
            id: WorkspaceAttachmentId::new(),
            workspace_id: WorkspaceId::new(),
            kind: WorkspaceAttachmentKind::ReferenceRepo,
            name: "ref".to_string(),
            source: "https://example.test/repo.git".to_string(),
            revision: Some("main".to_string()),
            subpath: None,
            mount_relpath: "refs/ref".to_string(),
            mode: AttachmentMode::Ro,
            update_policy: AttachmentUpdatePolicy::Manual,
            status: WorkspaceAttachmentStatus::Pending,
            last_sync_at: None,
            error_message: Some("waiting".to_string()),
            created_at: now,
            updated_at: now,
        };
        assert_same_json(
            WorkspaceAttachmentRouteResponse::from(attachment.clone()),
            attachment,
        );
    }

    #[test]
    fn active_workspace_route_wrappers_match_active_wire_shape() {
        let workspace_id = WorkspaceId::new();
        let snapshot = WorkspaceActiveSnapshot {
            workspace_id,
            snapshot_rev: 7,
            archived_rev: 3,
            active: ctx_core::models::WorkspaceActivePage {
                tasks: Vec::new(),
                total_count: 0,
            },
        };
        assert_same_json(
            WorkspaceActiveSnapshotRouteResponse::from(snapshot.clone()),
            snapshot,
        );

        let heads = WorkspaceActiveHeadBatch {
            workspace_id,
            snapshot_rev: 7,
            heads: Vec::new(),
        };
        assert_same_json(
            WorkspaceActiveHeadBatchRouteResponse::from(heads.clone()),
            heads,
        );
    }

    #[test]
    fn harness_container_route_response_matches_container_status_wire_shape() {
        let status = WorkspaceContainerStatus {
            name: "ctx-harness".to_string(),
            running: true,
            known: true,
            mount_mode: Some(ContainerMountMode::DiskIsolated),
            network_mode: Some(ContainerNetworkMode::Allowlist),
            allowlist: vec!["api.example.test".to_string()],
            egress_guard: Some(true),
        };
        assert_same_json(
            WorkspaceHarnessContainerStatusRouteResponse::from(status.clone()),
            status,
        );
        assert_same_json(
            Option::<WorkspaceHarnessContainerStatusRouteResponse>::None,
            Option::<WorkspaceContainerStatus>::None,
        );
    }

    #[test]
    fn workspace_route_params_parse_invalid_ids_to_route_errors() {
        let workspace = WorkspaceRouteParams::new("not-a-workspace")
            .parse_workspace_id()
            .unwrap_err();
        assert_eq!(workspace.kind(), WorkspaceRouteErrorKind::BadRequest);
        assert_eq!(workspace.message(), "invalid workspace id");

        let worktree = WorktreeRouteParams::new("not-a-worktree")
            .parse_worktree_id()
            .unwrap_err();
        assert_eq!(worktree.kind(), WorkspaceRouteErrorKind::BadRequest);
        assert_eq!(worktree.message(), "invalid worktree id");
    }

    #[test]
    fn workspace_route_error_helpers_preserve_status_classes() {
        let hydration = workspace_hydration_route_error(WorkspaceHydrationError::NotFound);
        assert_eq!(hydration.kind(), WorkspaceRouteErrorKind::NotFound);
        assert_eq!(hydration.message(), "workspace not found");

        let deletion = workspace_delete_route_error(WorkspaceDeleteError::NotFound);
        assert_eq!(deletion.kind(), WorkspaceRouteErrorKind::NotFound);
        assert_eq!(deletion.message(), "workspace not found");

        let download = route_file_download_error(RouteFileDownloadError::NotFound);
        assert_eq!(download.kind(), WorkspaceRouteErrorKind::NotFound);

        let harness_status = workspace_harness_container_status_error(
            WorkspaceHarnessContainerError::ExecutionSettings(
                EffectiveExecutionSettingsError::Internal(anyhow::anyhow!("settings failed")),
            ),
        );
        assert_eq!(harness_status.kind(), WorkspaceRouteErrorKind::Internal);

        let harness_ensure = workspace_harness_container_ensure_error(
            WorkspaceHarnessContainerError::Ensure(anyhow::anyhow!("bad container request")),
        );
        assert_eq!(harness_ensure.kind(), WorkspaceRouteErrorKind::BadRequest);
        assert_eq!(harness_ensure.message(), "bad container request");
    }

    #[test]
    fn workspace_file_completions_query_preserves_http_query_shape() {
        let empty: WorkspaceFileCompletionsRouteQuery =
            serde_json::from_value(serde_json::json!({})).expect("empty query shape");
        assert_eq!(
            empty,
            WorkspaceFileCompletionsRouteQuery {
                query: None,
                limit: None,
            }
        );

        let populated: WorkspaceFileCompletionsRouteQuery =
            serde_json::from_value(serde_json::json!({
                "query": "src",
                "limit": 25,
            }))
            .expect("populated query shape");
        assert_eq!(
            populated,
            WorkspaceFileCompletionsRouteQuery {
                query: Some("src".to_string()),
                limit: Some(25),
            }
        );
    }

    #[test]
    fn workspace_file_completion_storage_errors_map_to_507_class() {
        let error = FileCompletionsError::from_internal_error(
            "resolving data plane",
            anyhow::anyhow!("No space left on device"),
        );
        let route_error = file_completions_route_error(error);
        assert_eq!(
            route_error.kind(),
            WorkspaceRouteErrorKind::InsufficientStorage
        );
    }
}
