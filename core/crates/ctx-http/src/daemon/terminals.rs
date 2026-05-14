use std::sync::Arc;

use chrono::{DateTime, Utc};
use ctx_core::ids::{TerminalId, WorkspaceId};
use ctx_core::models::TerminalSession;
use ctx_transport_runtime::terminal_launch::TerminalLaunchError;
use ctx_transport_runtime::terminals::TerminalSessionHandle;

use crate::daemon::{DaemonState, TransportHandle};

mod launch;

pub(crate) use launch::CreateTerminalLaunchRequest;

pub(crate) async fn list_workspace_terminals(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
) -> Vec<TerminalSession> {
    state.transport.terminals.list(workspace_id).await
}

pub(crate) async fn create_workspace_terminal(
    state: &Arc<DaemonState>,
    req: CreateTerminalLaunchRequest,
) -> Result<TerminalSession, TerminalLaunchError> {
    launch::create_workspace_terminal(state, req).await
}

pub(crate) async fn delete_terminal(state: &Arc<DaemonState>, terminal_id: TerminalId) -> bool {
    let session = state.transport.terminals.remove(terminal_id).await;
    if let Some(session) = session {
        let _ = session.kill();
        session.mark_exited(None);
        return true;
    }
    false
}

pub(crate) struct TerminalStreamConnectPath {
    pub(crate) stream_path: String,
    pub(crate) expires_at: DateTime<Utc>,
}

pub(crate) async fn mint_terminal_stream_token(
    state: &Arc<DaemonState>,
    terminal_id: TerminalId,
) -> Option<TerminalStreamConnectPath> {
    let handle = state.transport.terminals.get(terminal_id).await?;
    let (stream_path, expires_at) = handle.issue_stream_connect_path();
    Some(TerminalStreamConnectPath {
        stream_path,
        expires_at,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TerminalStreamAccessError {
    MissingToken,
    NotFound,
    Unauthorized,
}

pub(crate) async fn require_terminal_stream_access(
    state: &Arc<DaemonState>,
    terminal_id: TerminalId,
    token: Option<&str>,
) -> Result<Arc<TerminalSessionHandle>, TerminalStreamAccessError> {
    let provided_token = token.ok_or(TerminalStreamAccessError::MissingToken)?;
    let handle = state
        .transport
        .terminals
        .get(terminal_id)
        .await
        .ok_or(TerminalStreamAccessError::NotFound)?;
    if !handle.consume_stream_token(provided_token) {
        return Err(TerminalStreamAccessError::Unauthorized);
    }
    Ok(handle)
}

impl TransportHandle {
    pub(crate) async fn list_workspace_terminals(
        &self,
        workspace_id: WorkspaceId,
    ) -> Vec<TerminalSession> {
        list_workspace_terminals(&self.state, workspace_id).await
    }

    pub(crate) async fn create_workspace_terminal(
        &self,
        req: CreateTerminalLaunchRequest,
    ) -> Result<TerminalSession, TerminalLaunchError> {
        create_workspace_terminal(&self.state, req).await
    }

    pub(crate) async fn delete_terminal(&self, terminal_id: TerminalId) -> bool {
        delete_terminal(&self.state, terminal_id).await
    }

    pub(crate) async fn mint_terminal_stream_token(
        &self,
        terminal_id: TerminalId,
    ) -> Option<TerminalStreamConnectPath> {
        mint_terminal_stream_token(&self.state, terminal_id).await
    }

    pub(crate) async fn require_terminal_stream_access(
        &self,
        terminal_id: TerminalId,
        token: Option<&str>,
    ) -> Result<Arc<TerminalSessionHandle>, TerminalStreamAccessError> {
        require_terminal_stream_access(&self.state, terminal_id, token).await
    }
}
