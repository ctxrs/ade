use std::sync::Arc;

use chrono::{DateTime, Utc};
use ctx_core::ids::{TerminalId, WorkspaceId};
use ctx_core::models::{TerminalSession, TerminalStatus};
use ctx_transport_runtime::terminal_launch::TerminalLaunchError;
use ctx_transport_runtime::terminals::{TerminalSessionHandle, TerminalStatusEvent};

use crate::daemon::{DaemonState, TransportHandle};

mod launch;
mod route_contract;

use self::launch::CreateTerminalLaunchRequest;

pub use route_contract::{
    CreateTerminalRouteRequest, DeleteTerminalRouteParams, ListWorkspaceTerminalsRouteParams,
    MintTerminalStreamTokenRouteParams, TerminalRouteError, TerminalRouteErrorKind,
    TerminalSessionRouteResponse, TerminalStatusRouteResponse, TerminalStreamConnectRouteResponse,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalStreamAccessError {
    MissingToken,
    NotFound,
    Unauthorized,
}

#[derive(Clone)]
pub struct TerminalStreamSession {
    handle: Arc<TerminalSessionHandle>,
}

pub struct TerminalStreamConnection {
    pub session: TerminalStreamSession,
    pub output_rx: TerminalStreamOutputReceiver,
    pub status_rx: TerminalStreamStatusReceiver,
    pub initial_snapshot: TerminalStreamInitialSnapshot,
    _client_guard: TerminalStreamClientGuard,
}

#[derive(Clone, Debug)]
pub struct TerminalStreamInitialSnapshot {
    pub status: TerminalStatus,
    pub exit_code: Option<i32>,
    pub output_tail: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct TerminalStreamStatusUpdate {
    pub status: TerminalStatus,
    pub exit_code: Option<i32>,
}

pub enum TerminalStreamOutputRecv {
    Bytes(Vec<u8>),
    Lagged,
    Closed,
}

pub enum TerminalStreamStatusRecv {
    Update(TerminalStreamStatusUpdate),
    Lagged,
    Closed,
}

pub struct TerminalStreamOutputReceiver {
    inner: tokio::sync::broadcast::Receiver<Vec<u8>>,
}

pub struct TerminalStreamStatusReceiver {
    inner: tokio::sync::broadcast::Receiver<TerminalStatusEvent>,
}

struct TerminalStreamClientGuard {
    handle: Arc<TerminalSessionHandle>,
}

impl Drop for TerminalStreamClientGuard {
    fn drop(&mut self) {
        self.handle.mark_client_disconnected();
    }
}

impl TerminalStreamSession {
    fn new(handle: Arc<TerminalSessionHandle>) -> Self {
        Self { handle }
    }

    pub fn connect(&self, tail_bytes: usize) -> TerminalStreamConnection {
        self.handle.mark_client_connected();
        let output_rx = TerminalStreamOutputReceiver {
            inner: self.handle.output_receiver(),
        };
        let status_rx = TerminalStreamStatusReceiver {
            inner: self.handle.status_receiver(),
        };
        TerminalStreamConnection {
            session: self.clone(),
            output_rx,
            status_rx,
            initial_snapshot: self.initial_snapshot(tail_bytes),
            _client_guard: TerminalStreamClientGuard {
                handle: Arc::clone(&self.handle),
            },
        }
    }

    pub fn initial_snapshot(&self, tail_bytes: usize) -> TerminalStreamInitialSnapshot {
        let snapshot = self.handle.snapshot();
        TerminalStreamInitialSnapshot {
            status: snapshot.status,
            exit_code: snapshot.exit_code,
            output_tail: self.output_tail(tail_bytes),
        }
    }

    pub fn output_tail(&self, tail_bytes: usize) -> Vec<u8> {
        self.handle.output_snapshot_tail(tail_bytes)
    }

    pub fn write_input(&self, data: Vec<u8>) {
        self.handle.send_input(data);
    }

    pub fn resize_terminal(&self, cols: u16, rows: u16) -> anyhow::Result<()> {
        self.handle.resize(cols, rows)
    }
}

impl TerminalStreamOutputReceiver {
    pub async fn recv(&mut self) -> TerminalStreamOutputRecv {
        match self.inner.recv().await {
            Ok(bytes) => TerminalStreamOutputRecv::Bytes(bytes),
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                TerminalStreamOutputRecv::Lagged
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                TerminalStreamOutputRecv::Closed
            }
        }
    }
}

impl TerminalStreamStatusReceiver {
    pub async fn recv(&mut self) -> TerminalStreamStatusRecv {
        match self.inner.recv().await {
            Ok(event) => TerminalStreamStatusRecv::Update(TerminalStreamStatusUpdate {
                status: event.status,
                exit_code: event.exit_code,
            }),
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                TerminalStreamStatusRecv::Lagged
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                TerminalStreamStatusRecv::Closed
            }
        }
    }
}

pub async fn require_terminal_stream_access(
    state: &Arc<DaemonState>,
    terminal_id: TerminalId,
    token: Option<&str>,
) -> Result<TerminalStreamSession, TerminalStreamAccessError> {
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
    Ok(TerminalStreamSession::new(handle))
}

impl TransportHandle {
    pub async fn require_terminal_stream_access(
        &self,
        terminal_id: TerminalId,
        token: Option<&str>,
    ) -> Result<TerminalStreamSession, TerminalStreamAccessError> {
        require_terminal_stream_access(&self.state, terminal_id, token).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_stream_initial_snapshot_clamps_tail() {
        let session =
            TerminalStreamSession::new(TerminalSessionHandle::test_handle_with_output(b"abcdef"));

        assert_eq!(session.initial_snapshot(0).output_tail, b"");
        assert_eq!(session.initial_snapshot(3).output_tail, b"def");
        assert_eq!(session.initial_snapshot(64).output_tail, b"abcdef");
    }

    #[tokio::test]
    async fn terminal_stream_status_receiver_maps_runtime_events() {
        let session =
            TerminalStreamSession::new(TerminalSessionHandle::test_handle_with_output(b""));
        let mut connection = session.connect(0);

        session.handle.mark_exited(Some(7));

        match connection.status_rx.recv().await {
            TerminalStreamStatusRecv::Update(update) => {
                assert!(matches!(update.status, TerminalStatus::Exited));
                assert_eq!(update.exit_code, Some(7));
            }
            TerminalStreamStatusRecv::Lagged | TerminalStreamStatusRecv::Closed => {
                panic!("expected status update")
            }
        }
    }
}
