use ctx_core::ids::WorkspaceId;

use crate::daemon::DaemonState;
use crate::daemon::WorkspaceStreamHandle;

#[derive(Debug)]
pub enum WorkspaceStreamAccessError {
    NotFound,
    Internal(anyhow::Error),
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct WorkspaceStreamRouteParams {
    workspace_id: String,
}

impl WorkspaceStreamRouteParams {
    pub fn new(workspace_id: impl Into<String>) -> Self {
        Self {
            workspace_id: workspace_id.into(),
        }
    }

    pub(super) fn parse_workspace_id(&self) -> Result<WorkspaceId, WorkspaceStreamRouteError> {
        uuid::Uuid::parse_str(&self.workspace_id)
            .map(WorkspaceId)
            .map_err(|_| WorkspaceStreamRouteError::bad_request("invalid workspace id"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceStreamRouteErrorKind {
    BadRequest,
    NotFound,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceStreamRouteError {
    kind: WorkspaceStreamRouteErrorKind,
    message: String,
}

impl WorkspaceStreamRouteError {
    pub(super) fn bad_request(message: impl Into<String>) -> Self {
        Self {
            kind: WorkspaceStreamRouteErrorKind::BadRequest,
            message: message.into(),
        }
    }

    pub(super) fn not_found(message: impl Into<String>) -> Self {
        Self {
            kind: WorkspaceStreamRouteErrorKind::NotFound,
            message: message.into(),
        }
    }

    pub(super) fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: WorkspaceStreamRouteErrorKind::Internal,
            message: message.into(),
        }
    }

    pub(super) fn from_stream_access(error: WorkspaceStreamAccessError) -> Self {
        match error {
            WorkspaceStreamAccessError::NotFound => Self::not_found("workspace not found"),
            WorkspaceStreamAccessError::Internal(error) => Self::internal(error.to_string()),
        }
    }

    pub fn kind(&self) -> WorkspaceStreamRouteErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceStreamRouteAdmission {
    workspace_id: WorkspaceId,
}

impl WorkspaceStreamRouteAdmission {
    pub(super) fn new(workspace_id: WorkspaceId) -> Self {
        Self { workspace_id }
    }

    pub fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }
}

pub(super) async fn require_existing_workspace_for_stream(
    state: &DaemonState,
    workspace_id: WorkspaceId,
) -> Result<(), WorkspaceStreamAccessError> {
    let exists = state
        .global_store()
        .get_workspace(workspace_id)
        .await
        .map_err(WorkspaceStreamAccessError::Internal)?
        .is_some();
    if !exists {
        return Err(WorkspaceStreamAccessError::NotFound);
    }
    Ok(())
}

impl WorkspaceStreamHandle {
    pub async fn workspace_exists(&self, workspace_id: WorkspaceId) -> anyhow::Result<bool> {
        self.state
            .global_store()
            .get_workspace(workspace_id)
            .await
            .map(|workspace| workspace.is_some())
    }

    pub async fn require_workspace_active_stream_access(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<(), WorkspaceStreamAccessError> {
        require_existing_workspace_for_stream(&self.state, workspace_id).await
    }

    pub async fn admit_workspace_active_stream_for_route(
        &self,
        params: WorkspaceStreamRouteParams,
    ) -> Result<WorkspaceStreamRouteAdmission, WorkspaceStreamRouteError> {
        let workspace_id = params.parse_workspace_id()?;
        self.require_workspace_active_stream_access(workspace_id)
            .await
            .map_err(WorkspaceStreamRouteError::from_stream_access)?;
        Ok(WorkspaceStreamRouteAdmission::new(workspace_id))
    }
}
