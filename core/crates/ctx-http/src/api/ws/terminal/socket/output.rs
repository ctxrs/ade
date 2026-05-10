use std::sync::{atomic::AtomicBool, Arc};

use axum::extract::ws::Message as WsMessage;
use ctx_transport_runtime::terminals::TerminalSessionHandle;
use tokio::sync::{broadcast, mpsc};

use super::super::queue::{
    queue_terminal_ws_message, queue_terminal_ws_tail_resync_if_requested,
    queue_terminal_ws_tail_snapshot, request_terminal_ws_tail_resync, TerminalWsQueueOutcome,
};

pub(super) async fn forward_terminal_output(
    mut output_rx: broadcast::Receiver<Vec<u8>>,
    event_tx: mpsc::Sender<WsMessage>,
    session: Arc<TerminalSessionHandle>,
    snapshot_tail: usize,
    needs_tail_resync: Arc<AtomicBool>,
) {
    loop {
        match output_rx.recv().await {
            Ok(bytes) => {
                if let Some(outcome) = queue_terminal_ws_tail_resync_if_requested(
                    &event_tx,
                    &session,
                    snapshot_tail,
                    needs_tail_resync.as_ref(),
                ) {
                    match outcome {
                        TerminalWsQueueOutcome::Enqueued => continue,
                        TerminalWsQueueOutcome::Dropped => {
                            tracing::debug!(
                                "dropping terminal tail resync for slow websocket consumer"
                            );
                            continue;
                        }
                        TerminalWsQueueOutcome::Closed => break,
                    }
                }
                match queue_terminal_ws_message(&event_tx, WsMessage::Binary(bytes)) {
                    TerminalWsQueueOutcome::Enqueued => {}
                    TerminalWsQueueOutcome::Dropped => {
                        request_terminal_ws_tail_resync(needs_tail_resync.as_ref());
                        tracing::debug!("dropping terminal output for slow websocket consumer");
                    }
                    TerminalWsQueueOutcome::Closed => break,
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => {
                match queue_terminal_ws_tail_snapshot(&event_tx, &session, snapshot_tail) {
                    TerminalWsQueueOutcome::Enqueued => {}
                    TerminalWsQueueOutcome::Dropped => {
                        request_terminal_ws_tail_resync(needs_tail_resync.as_ref());
                        tracing::debug!(
                            "dropping terminal tail resync for slow websocket consumer"
                        );
                    }
                    TerminalWsQueueOutcome::Closed => break,
                }
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}
