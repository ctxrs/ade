use std::sync::Arc;

use chrono::{DateTime, Utc};
use ctx_core::ids::{TerminalId, WorkspaceId};
use ctx_core::models::TerminalSession;
use ctx_transport_runtime::terminal_launch::TerminalLaunchError;

use crate::daemon::AppState;

mod launch;

pub(crate) use launch::CreateTerminalLaunchRequest;

pub(crate) async fn list_workspace_terminals(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
) -> Vec<TerminalSession> {
    state.transport.terminals.list(workspace_id).await
}

pub(crate) async fn create_workspace_terminal(
    state: &Arc<AppState>,
    req: CreateTerminalLaunchRequest,
) -> Result<TerminalSession, TerminalLaunchError> {
    launch::create_workspace_terminal(state, req).await
}

pub(crate) async fn delete_terminal(state: &Arc<AppState>, terminal_id: TerminalId) -> bool {
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
    state: &Arc<AppState>,
    terminal_id: TerminalId,
) -> Option<TerminalStreamConnectPath> {
    let handle = state.transport.terminals.get(terminal_id).await?;
    let (stream_path, expires_at) = handle.issue_stream_connect_path();
    Some(TerminalStreamConnectPath {
        stream_path,
        expires_at,
    })
}
