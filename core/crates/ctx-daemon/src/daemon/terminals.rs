use std::sync::Arc;

use chrono::{DateTime, Utc};
use ctx_core::ids::{TerminalId, WorkspaceId};
use ctx_core::models::TerminalSession;
use ctx_transport_runtime::terminal_launch::TerminalLaunchError;

use crate::daemon::DaemonState;

mod launch;
mod route_contract;

use self::launch::CreateTerminalLaunchRequest;

pub use route_contract::{
    CreateTerminalRouteRequest, DeleteTerminalRouteParams, ListWorkspaceTerminalsRouteParams,
    MintTerminalStreamTokenRouteParams, TerminalRouteError, TerminalRouteErrorKind,
    TerminalSessionRouteResponse, TerminalStatusRouteResponse, TerminalStreamConnectRouteResponse,
    TerminalStreamRouteAdmission, TerminalStreamRouteParams,
};

async fn list_workspace_terminals(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
) -> Vec<TerminalSession> {
    state.transport.terminals.list(workspace_id).await
}

async fn create_workspace_terminal(
    state: &Arc<DaemonState>,
    req: CreateTerminalLaunchRequest,
) -> Result<TerminalSession, TerminalLaunchError> {
    launch::create_workspace_terminal(state, req).await
}

async fn delete_terminal(state: &Arc<DaemonState>, terminal_id: TerminalId) -> bool {
    let session = state.transport.terminals.remove(terminal_id).await;
    if let Some(session) = session {
        let _ = session.kill();
        session.mark_exited(None);
        return true;
    }
    false
}

pub struct TerminalStreamConnectPath {
    pub stream_path: String,
    pub expires_at: DateTime<Utc>,
}

async fn mint_terminal_stream_token(
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
