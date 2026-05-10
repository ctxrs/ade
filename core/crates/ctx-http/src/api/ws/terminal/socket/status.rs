use std::sync::{atomic::AtomicBool, Arc};

use axum::extract::ws::Message as WsMessage;
use ctx_transport_runtime::terminals::{TerminalServerMessage, TerminalStatusEvent};
use tokio::sync::{broadcast, mpsc};

use super::super::queue::{
    queue_terminal_ws_message, request_terminal_ws_tail_resync, TerminalWsQueueOutcome,
};

pub(super) async fn forward_terminal_status(
    mut status_rx: broadcast::Receiver<TerminalStatusEvent>,
    event_tx: mpsc::Sender<WsMessage>,
    needs_tail_resync: Arc<AtomicBool>,
) {
    loop {
        match status_rx.recv().await {
            Ok(ev) => {
                let payload = serde_json::to_string(&TerminalServerMessage::Status {
                    status: ev.status,
                    exit_code: ev.exit_code,
                })
                .unwrap_or_else(|_| "{\"type\":\"status\",\"status\":\"exited\"}".to_string());
                match queue_terminal_ws_message(&event_tx, WsMessage::Text(payload)) {
                    TerminalWsQueueOutcome::Enqueued => {}
                    TerminalWsQueueOutcome::Dropped => {
                        tracing::debug!("dropping terminal status for slow websocket consumer");
                    }
                    TerminalWsQueueOutcome::Closed => break,
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => {
                request_terminal_ws_tail_resync(needs_tail_resync.as_ref());
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}
