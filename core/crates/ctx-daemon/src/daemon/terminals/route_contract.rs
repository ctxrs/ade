use chrono::{DateTime, Utc};
use ctx_core::ids::{SessionId, TaskId, TerminalId, WorkspaceId, WorktreeId};
use ctx_core::models::{TerminalSession, TerminalStatus};
use ctx_transport_runtime::terminal_launch::{TerminalLaunchError, TerminalLaunchErrorKind};
use serde::{Deserialize, Serialize};

use crate::daemon::TransportHandle;

use super::{launch::CreateTerminalLaunchRequest, TerminalStreamConnectPath};

#[derive(Debug)]
pub struct ListWorkspaceTerminalsRouteParams {
    workspace_id: String,
}

impl ListWorkspaceTerminalsRouteParams {
    pub fn new(workspace_id: impl Into<String>) -> Self {
        Self {
            workspace_id: workspace_id.into(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateTerminalRouteRequest {
    #[serde(default)]
    task_id: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    worktree_id: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    shell: Option<String>,
}

#[derive(Debug)]
pub struct DeleteTerminalRouteParams {
    terminal_id: String,
}

impl DeleteTerminalRouteParams {
    pub fn new(terminal_id: impl Into<String>) -> Self {
        Self {
            terminal_id: terminal_id.into(),
        }
    }
}

#[derive(Debug)]
pub struct MintTerminalStreamTokenRouteParams {
    terminal_id: String,
}

impl MintTerminalStreamTokenRouteParams {
    pub fn new(terminal_id: impl Into<String>) -> Self {
        Self {
            terminal_id: terminal_id.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TerminalSessionRouteResponse {
    pub id: TerminalId,
    pub workspace_id: WorkspaceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<TaskId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<WorktreeId>,
    pub cwd: String,
    pub shell: String,
    pub title: String,
    pub status: TerminalStatusRouteResponse,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub stream_path: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalStatusRouteResponse {
    Running,
    Exited,
}

impl From<TerminalSession> for TerminalSessionRouteResponse {
    fn from(session: TerminalSession) -> Self {
        Self {
            id: session.id,
            workspace_id: session.workspace_id,
            task_id: session.task_id,
            session_id: session.session_id,
            worktree_id: session.worktree_id,
            cwd: session.cwd,
            shell: session.shell,
            title: session.title,
            status: session.status.into(),
            exit_code: session.exit_code,
            stream_path: session.stream_path,
            created_at: session.created_at,
            updated_at: session.updated_at,
        }
    }
}

impl From<TerminalStatus> for TerminalStatusRouteResponse {
    fn from(status: TerminalStatus) -> Self {
        match status {
            TerminalStatus::Running => Self::Running,
            TerminalStatus::Exited => Self::Exited,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TerminalStreamConnectRouteResponse {
    pub stream_path: String,
    pub expires_at: DateTime<Utc>,
}

impl From<TerminalStreamConnectPath> for TerminalStreamConnectRouteResponse {
    fn from(token: TerminalStreamConnectPath) -> Self {
        Self {
            stream_path: token.stream_path,
            expires_at: token.expires_at,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalRouteErrorKind {
    BadRequest,
    NotFound,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalRouteError {
    kind: TerminalRouteErrorKind,
    message: String,
}

impl TerminalRouteError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            kind: TerminalRouteErrorKind::BadRequest,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            kind: TerminalRouteErrorKind::NotFound,
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: TerminalRouteErrorKind::Internal,
            message: message.into(),
        }
    }

    fn from_launch_error(error: TerminalLaunchError) -> Self {
        match error.kind() {
            TerminalLaunchErrorKind::BadRequest => Self::bad_request(error.message()),
            TerminalLaunchErrorKind::NotFound => Self::not_found(error.message()),
            TerminalLaunchErrorKind::Internal => Self::internal(error.message()),
        }
    }

    pub fn kind(&self) -> TerminalRouteErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl CreateTerminalRouteRequest {
    fn into_launch_request(
        self,
        raw_workspace_id: &str,
    ) -> Result<CreateTerminalLaunchRequest, TerminalRouteError> {
        let workspace_id = WorkspaceId(
            uuid::Uuid::parse_str(raw_workspace_id)
                .map_err(|_| TerminalRouteError::bad_request("invalid workspace id"))?,
        );
        let task_id = parse_optional_id(self.task_id, "invalid task_id")?.map(TaskId);
        let session_id = parse_optional_id(self.session_id, "invalid session_id")?.map(SessionId);
        let worktree_id =
            parse_optional_id(self.worktree_id, "invalid worktree_id")?.map(WorktreeId);

        Ok(CreateTerminalLaunchRequest {
            workspace_id,
            task_id,
            session_id,
            worktree_id,
            cwd: self.cwd,
            shell: self.shell,
        })
    }
}

impl TransportHandle {
    pub async fn list_workspace_terminal_responses_for_route(
        &self,
        params: ListWorkspaceTerminalsRouteParams,
    ) -> Result<Vec<TerminalSessionRouteResponse>, TerminalRouteError> {
        let workspace_id = parse_workspace_id(&params.workspace_id)?;
        let sessions = super::list_workspace_terminals(&self.state, workspace_id).await;
        Ok(sessions.into_iter().map(Into::into).collect())
    }

    pub async fn create_workspace_terminal_for_route(
        &self,
        raw_workspace_id: &str,
        req: CreateTerminalRouteRequest,
    ) -> Result<TerminalSessionRouteResponse, TerminalRouteError> {
        let launch_req = req.into_launch_request(raw_workspace_id)?;
        super::create_workspace_terminal(&self.state, launch_req)
            .await
            .map(Into::into)
            .map_err(TerminalRouteError::from_launch_error)
    }

    pub async fn delete_terminal_for_route(
        &self,
        params: DeleteTerminalRouteParams,
    ) -> Result<(), TerminalRouteError> {
        let terminal_id = parse_terminal_id(&params.terminal_id)?;
        if super::delete_terminal(&self.state, terminal_id).await {
            return Ok(());
        }
        Err(TerminalRouteError::not_found("terminal not found"))
    }

    pub async fn mint_terminal_stream_token_for_route(
        &self,
        params: MintTerminalStreamTokenRouteParams,
    ) -> Result<TerminalStreamConnectRouteResponse, TerminalRouteError> {
        let terminal_id = parse_terminal_id(&params.terminal_id)?;
        super::mint_terminal_stream_token(&self.state, terminal_id)
            .await
            .map(Into::into)
            .ok_or_else(|| TerminalRouteError::not_found("terminal not found"))
    }
}

fn parse_workspace_id(value: &str) -> Result<WorkspaceId, TerminalRouteError> {
    uuid::Uuid::parse_str(value)
        .map(WorkspaceId)
        .map_err(|_| TerminalRouteError::bad_request("invalid workspace id"))
}

fn parse_terminal_id(value: &str) -> Result<TerminalId, TerminalRouteError> {
    uuid::Uuid::parse_str(value)
        .map(TerminalId)
        .map_err(|_| TerminalRouteError::bad_request("invalid terminal id"))
}

fn parse_optional_id(
    raw: Option<String>,
    error: &'static str,
) -> Result<Option<uuid::Uuid>, TerminalRouteError> {
    raw.map(|value| {
        uuid::Uuid::parse_str(value.trim()).map_err(|_| TerminalRouteError::bad_request(error))
    })
    .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    fn terminal_session_with_optional_ids(status: TerminalStatus) -> TerminalSession {
        TerminalSession {
            id: TerminalId::new(),
            workspace_id: WorkspaceId::new(),
            task_id: Some(TaskId::new()),
            session_id: Some(SessionId::new()),
            worktree_id: Some(WorktreeId::new()),
            cwd: "/tmp/work".to_string(),
            shell: "/bin/zsh".to_string(),
            title: "zsh".to_string(),
            status,
            exit_code: Some(7),
            stream_path: "/api/terminals/stream?token=secret".to_string(),
            created_at: Utc.with_ymd_and_hms(2026, 5, 17, 10, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 5, 17, 10, 1, 0).unwrap(),
        }
    }

    #[test]
    fn terminal_route_response_matches_raw_session_wire_shape_with_optional_fields() {
        let session = terminal_session_with_optional_ids(TerminalStatus::Running);
        let response = TerminalSessionRouteResponse::from(session.clone());

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            serde_json::to_value(session).unwrap()
        );
    }

    #[test]
    fn terminal_route_response_matches_raw_session_wire_shape_without_optional_fields() {
        let mut session = terminal_session_with_optional_ids(TerminalStatus::Exited);
        session.task_id = None;
        session.session_id = None;
        session.worktree_id = None;
        session.exit_code = None;
        let response = TerminalSessionRouteResponse::from(session.clone());

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            serde_json::to_value(session).unwrap()
        );
    }

    #[test]
    fn stream_connect_route_response_preserves_wire_shape() {
        let expires_at = Utc.with_ymd_and_hms(2026, 5, 17, 11, 0, 0).unwrap();
        let response = TerminalStreamConnectRouteResponse::from(TerminalStreamConnectPath {
            stream_path: "/api/terminals/abc/stream?token=secret".to_string(),
            expires_at,
        });

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            json!({
                "stream_path": "/api/terminals/abc/stream?token=secret",
                "expires_at": expires_at,
            })
        );
    }

    #[test]
    fn create_route_request_preserves_invalid_id_messages() {
        let error = CreateTerminalRouteRequest {
            task_id: None,
            session_id: None,
            worktree_id: None,
            cwd: None,
            shell: None,
        }
        .into_launch_request("not-a-workspace")
        .unwrap_err();
        assert_eq!(error.kind(), TerminalRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "invalid workspace id");

        let workspace_id = WorkspaceId::new().0.to_string();
        for (field, message) in [
            ("task", "invalid task_id"),
            ("session", "invalid session_id"),
            ("worktree", "invalid worktree_id"),
        ] {
            let req = CreateTerminalRouteRequest {
                task_id: (field == "task").then(|| "not-a-task".to_string()),
                session_id: (field == "session").then(|| "not-a-session".to_string()),
                worktree_id: (field == "worktree").then(|| "not-a-worktree".to_string()),
                cwd: None,
                shell: None,
            };
            let error = req.into_launch_request(&workspace_id).unwrap_err();
            assert_eq!(error.kind(), TerminalRouteErrorKind::BadRequest);
            assert_eq!(error.message(), message);
        }
    }

    #[test]
    fn create_route_request_trims_optional_ids() {
        let workspace_id = WorkspaceId::new();
        let task_id = TaskId::new();
        let session_id = SessionId::new();
        let worktree_id = WorktreeId::new();
        let req = CreateTerminalRouteRequest {
            task_id: Some(format!(" {} ", task_id.0)),
            session_id: Some(format!("\n{}\t", session_id.0)),
            worktree_id: Some(format!(" {} ", worktree_id.0)),
            cwd: Some(".".to_string()),
            shell: Some("/bin/sh".to_string()),
        };

        let launch = req
            .into_launch_request(&workspace_id.0.to_string())
            .expect("valid ids should parse");

        assert_eq!(launch.workspace_id, workspace_id);
        assert_eq!(launch.task_id, Some(task_id));
        assert_eq!(launch.session_id, Some(session_id));
        assert_eq!(launch.worktree_id, Some(worktree_id));
        assert_eq!(launch.cwd.as_deref(), Some("."));
        assert_eq!(launch.shell.as_deref(), Some("/bin/sh"));
    }
}
