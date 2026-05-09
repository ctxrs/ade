use std::sync::atomic::{AtomicBool, Ordering};

use axum::extract::ws::Message as WsMessage;
use ctx_transport_runtime::terminals::TerminalSessionHandle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::api::ws) enum TerminalWsQueueOutcome {
    Enqueued,
    Dropped,
    Closed,
}

pub(in crate::api::ws) fn queue_terminal_ws_message(
    event_tx: &tokio::sync::mpsc::Sender<WsMessage>,
    msg: WsMessage,
) -> TerminalWsQueueOutcome {
    match event_tx.try_send(msg) {
        Ok(()) => TerminalWsQueueOutcome::Enqueued,
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => TerminalWsQueueOutcome::Dropped,
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => TerminalWsQueueOutcome::Closed,
    }
}

pub(super) fn queue_terminal_ws_tail_snapshot(
    event_tx: &tokio::sync::mpsc::Sender<WsMessage>,
    session: &TerminalSessionHandle,
    snapshot_tail: usize,
) -> TerminalWsQueueOutcome {
    let snapshot = session.output_snapshot_tail(snapshot_tail);
    if snapshot.is_empty() {
        return TerminalWsQueueOutcome::Enqueued;
    }
    queue_terminal_ws_message(event_tx, WsMessage::Binary(snapshot))
}

pub(super) fn request_terminal_ws_tail_resync(needs_tail_resync: &AtomicBool) {
    needs_tail_resync.store(true, Ordering::Release);
}

pub(in crate::api::ws) fn queue_terminal_ws_tail_resync_if_requested(
    event_tx: &tokio::sync::mpsc::Sender<WsMessage>,
    session: &TerminalSessionHandle,
    snapshot_tail: usize,
    needs_tail_resync: &AtomicBool,
) -> Option<TerminalWsQueueOutcome> {
    if !needs_tail_resync.swap(false, Ordering::AcqRel) {
        return None;
    }
    let outcome = queue_terminal_ws_tail_snapshot(event_tx, session, snapshot_tail);
    if matches!(outcome, TerminalWsQueueOutcome::Dropped) {
        request_terminal_ws_tail_resync(needs_tail_resync);
    }
    Some(outcome)
}
