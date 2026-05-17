use chrono::{DateTime, Utc};
use ctx_core::ids::{MergeQueueEntryId, SessionId, WorkspaceId, WorktreeId};
use ctx_core::models::{MergeQueueEntry, MergeQueueEntryStatus, MergeQueuePatchSource};
use serde::{Deserialize, Serialize};

use crate::daemon::{WorkspaceStoreAccessError, WorkspacesHandle};

#[derive(Debug, Deserialize)]
pub struct ListMergeQueueEntriesRouteRequest {
    workspace_id: String,
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct MergeQueueEntryRouteParams {
    workspace_id: String,
    id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MergeQueueEntryRouteResponse {
    pub id: MergeQueueEntryId,
    pub workspace_id: WorkspaceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<WorktreeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    pub target_branch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub patch_source: MergeQueuePatchSourceRouteResponse,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_commit_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_commit_sha: Option<String>,
    pub patch_path: String,
    pub patch_size: i64,
    pub status: MergeQueueEntryStatusRouteResponse,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_commit_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeQueuePatchSourceRouteResponse {
    Generated,
    Provided,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeQueueEntryStatusRouteResponse {
    Queued,
    Running,
    Passed,
    Failed,
    Conflict,
    Cancelled,
}

impl From<MergeQueueEntry> for MergeQueueEntryRouteResponse {
    fn from(entry: MergeQueueEntry) -> Self {
        Self {
            id: entry.id,
            workspace_id: entry.workspace_id,
            worktree_id: entry.worktree_id,
            session_id: entry.session_id,
            target_branch: entry.target_branch,
            message: entry.message,
            patch_source: entry.patch_source.into(),
            base_commit_sha: entry.base_commit_sha,
            head_commit_sha: entry.head_commit_sha,
            patch_path: entry.patch_path,
            patch_size: entry.patch_size,
            status: entry.status.into(),
            result_commit_sha: entry.result_commit_sha,
            error_message: entry.error_message,
            created_at: entry.created_at,
            updated_at: entry.updated_at,
        }
    }
}

impl From<MergeQueuePatchSource> for MergeQueuePatchSourceRouteResponse {
    fn from(source: MergeQueuePatchSource) -> Self {
        match source {
            MergeQueuePatchSource::Generated => Self::Generated,
            MergeQueuePatchSource::Provided => Self::Provided,
        }
    }
}

impl From<MergeQueueEntryStatus> for MergeQueueEntryStatusRouteResponse {
    fn from(status: MergeQueueEntryStatus) -> Self {
        match status {
            MergeQueueEntryStatus::Queued => Self::Queued,
            MergeQueueEntryStatus::Running => Self::Running,
            MergeQueueEntryStatus::Passed => Self::Passed,
            MergeQueueEntryStatus::Failed => Self::Failed,
            MergeQueueEntryStatus::Conflict => Self::Conflict,
            MergeQueueEntryStatus::Cancelled => Self::Cancelled,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum MergeQueueEntryRouteErrorKind {
    BadRequest,
    Internal,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct MergeQueueEntryRouteError {
    kind: MergeQueueEntryRouteErrorKind,
    message: String,
}

impl MergeQueueEntryRouteError {
    fn new(kind: MergeQueueEntryRouteErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(MergeQueueEntryRouteErrorKind::BadRequest, message)
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::new(MergeQueueEntryRouteErrorKind::Internal, message)
    }

    pub fn kind(&self) -> MergeQueueEntryRouteErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl WorkspacesHandle {
    pub async fn list_merge_queue_entry_responses_for_route(
        &self,
        req: ListMergeQueueEntriesRouteRequest,
    ) -> Result<Vec<MergeQueueEntryRouteResponse>, MergeQueueEntryRouteError> {
        let workspace_id = parse_workspace_id(&req.workspace_id)?;
        let store = self
            .existing_workspace_store(workspace_id)
            .await
            .map_err(list_store_error)?;
        let entries = store
            .list_merge_queue_entries(workspace_id, req.limit)
            .await
            .map_err(|error| MergeQueueEntryRouteError::internal(error.to_string()))?;
        Ok(entries.into_iter().map(Into::into).collect())
    }

    pub async fn cancel_merge_queue_entry_for_route(
        &self,
        params: MergeQueueEntryRouteParams,
    ) -> Result<MergeQueueEntryRouteResponse, MergeQueueEntryRouteError> {
        let (workspace_id, entry_id) = parse_entry_route_params(&params)?;
        self.cancel_merge_queue_entry(workspace_id, entry_id)
            .await
            .map(Into::into)
            .map_err(|error| MergeQueueEntryRouteError::bad_request(error.to_string()))
    }

    pub async fn retry_merge_queue_entry_for_route(
        &self,
        params: MergeQueueEntryRouteParams,
    ) -> Result<MergeQueueEntryRouteResponse, MergeQueueEntryRouteError> {
        let (workspace_id, entry_id) = parse_entry_route_params(&params)?;
        self.retry_merge_queue_entry(workspace_id, entry_id)
            .await
            .map(Into::into)
            .map_err(|error| MergeQueueEntryRouteError::bad_request(error.to_string()))
    }
}

fn parse_entry_route_params(
    params: &MergeQueueEntryRouteParams,
) -> Result<(WorkspaceId, MergeQueueEntryId), MergeQueueEntryRouteError> {
    Ok((
        parse_workspace_id(&params.workspace_id)?,
        parse_entry_id(&params.id)?,
    ))
}

fn parse_workspace_id(value: &str) -> Result<WorkspaceId, MergeQueueEntryRouteError> {
    uuid::Uuid::parse_str(value)
        .map(WorkspaceId)
        .map_err(|_| MergeQueueEntryRouteError::bad_request("invalid workspace id"))
}

fn parse_entry_id(value: &str) -> Result<MergeQueueEntryId, MergeQueueEntryRouteError> {
    uuid::Uuid::parse_str(value)
        .map(MergeQueueEntryId)
        .map_err(|_| MergeQueueEntryRouteError::bad_request("invalid entry id"))
}

fn list_store_error(error: WorkspaceStoreAccessError) -> MergeQueueEntryRouteError {
    match error {
        WorkspaceStoreAccessError::NotFound => {
            MergeQueueEntryRouteError::internal("workspace not found")
        }
        WorkspaceStoreAccessError::Unavailable(error) => {
            MergeQueueEntryRouteError::internal(error.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn full_entry() -> MergeQueueEntry {
        MergeQueueEntry {
            id: MergeQueueEntryId::new(),
            workspace_id: WorkspaceId::new(),
            worktree_id: Some(WorktreeId::new()),
            session_id: Some(SessionId::new()),
            target_branch: "dev".to_string(),
            message: Some("ship it".to_string()),
            patch_source: MergeQueuePatchSource::Provided,
            base_commit_sha: Some("base".to_string()),
            head_commit_sha: Some("head".to_string()),
            patch_path: "/tmp/entry.patch".to_string(),
            patch_size: 42,
            status: MergeQueueEntryStatus::Passed,
            result_commit_sha: Some("result".to_string()),
            error_message: Some("kept for wire-shape coverage".to_string()),
            created_at: Utc.with_ymd_and_hms(2026, 5, 17, 10, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 5, 17, 10, 1, 0).unwrap(),
        }
    }

    #[test]
    fn entry_route_response_matches_raw_entry_wire_shape_with_optional_fields() {
        let entry = full_entry();
        let response = MergeQueueEntryRouteResponse::from(entry.clone());

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            serde_json::to_value(entry).unwrap()
        );
    }

    #[test]
    fn entry_route_response_matches_raw_entry_wire_shape_without_optional_fields() {
        let mut entry = full_entry();
        entry.worktree_id = None;
        entry.session_id = None;
        entry.message = None;
        entry.base_commit_sha = None;
        entry.head_commit_sha = None;
        entry.result_commit_sha = None;
        entry.error_message = None;
        entry.patch_source = MergeQueuePatchSource::Generated;
        entry.status = MergeQueueEntryStatus::Queued;

        let response = MergeQueueEntryRouteResponse::from(entry.clone());

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            serde_json::to_value(entry).unwrap()
        );
    }

    #[test]
    fn entry_route_params_preserve_invalid_id_messages() {
        let error = parse_workspace_id("not-a-workspace").unwrap_err();
        assert_eq!(error.kind(), MergeQueueEntryRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "invalid workspace id");

        let error = parse_entry_id("not-an-entry").unwrap_err();
        assert_eq!(error.kind(), MergeQueueEntryRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "invalid entry id");
    }
}
