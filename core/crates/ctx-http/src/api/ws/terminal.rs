use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

// Terminal bytes are lossy; a slow browser should not force unbounded per-connection buffering.
const TERMINAL_WS_EVENT_QUEUE_LIMIT: usize = 128;
const TERMINAL_PING_INTERVAL: Duration = Duration::from_secs(25);
const TERMINAL_TAIL_RESYNC_INTERVAL: Duration = Duration::from_millis(25);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TerminalWsQueueOutcome {
    Enqueued,
    Dropped,
    Closed,
}

async fn require_terminal_stream_access(
    manager: &Arc<crate::terminals::TerminalManager>,
    id: TerminalId,
    token: Option<&str>,
) -> Result<Arc<crate::terminals::TerminalSessionHandle>, StatusCode> {
    let provided_token = token.ok_or(StatusCode::UNAUTHORIZED)?;
    let handle = manager.get(id).await.ok_or(StatusCode::NOT_FOUND)?;
    if !handle.matches_stream_token(provided_token) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(handle)
}

pub(in crate::api) async fn terminal_stream_ws(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    ws: WebSocketUpgrade,
) -> Result<Response, StatusCode> {
    let terminal_id = TerminalId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let session = require_terminal_stream_access(
        &state.transport.terminals,
        terminal_id,
        params.get("token").map(String::as_str),
    )
    .await?;

    let tail_bytes = terminal_stream_tail_bytes(&params);
    Ok(ws.on_upgrade(move |socket| async move {
        handle_terminal_socket(socket, session, tail_bytes).await;
    }))
}

fn terminal_stream_tail_bytes(params: &HashMap<String, String>) -> usize {
    params
        .get("tail")
        .and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                trimmed.parse::<usize>().ok()
            }
        })
        .unwrap_or(crate::terminals::DEFAULT_OUTPUT_TAIL_BYTES)
}

pub(super) fn queue_terminal_ws_message(
    event_tx: &tokio::sync::mpsc::Sender<WsMessage>,
    msg: WsMessage,
) -> TerminalWsQueueOutcome {
    match event_tx.try_send(msg) {
        Ok(()) => TerminalWsQueueOutcome::Enqueued,
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => TerminalWsQueueOutcome::Dropped,
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => TerminalWsQueueOutcome::Closed,
    }
}

fn queue_terminal_ws_tail_snapshot(
    event_tx: &tokio::sync::mpsc::Sender<WsMessage>,
    session: &crate::terminals::TerminalSessionHandle,
    snapshot_tail: usize,
) -> TerminalWsQueueOutcome {
    let snapshot = session.output_snapshot_tail(snapshot_tail);
    if snapshot.is_empty() {
        return TerminalWsQueueOutcome::Enqueued;
    }
    queue_terminal_ws_message(event_tx, WsMessage::Binary(snapshot))
}

fn request_terminal_ws_tail_resync(needs_tail_resync: &AtomicBool) {
    needs_tail_resync.store(true, Ordering::Release);
}

pub(super) fn queue_terminal_ws_tail_resync_if_requested(
    event_tx: &tokio::sync::mpsc::Sender<WsMessage>,
    session: &crate::terminals::TerminalSessionHandle,
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

async fn handle_terminal_socket(
    mut socket: WebSocket,
    session: Arc<crate::terminals::TerminalSessionHandle>,
    snapshot_tail: usize,
) {
    session.mark_client_connected();
    // Subscribe before capturing the snapshot tail so reconnecting clients do not
    // miss bytes emitted during the status/tail handshake window. This may replay
    // a small overlap from the tail, but it preserves contiguous output delivery.
    let mut output_rx = session.output_receiver();
    let mut status_rx = session.status_receiver();

    let snapshot = session.snapshot();
    let status_payload = serde_json::to_string(&TerminalServerMessage::Status {
        status: snapshot.status.clone(),
        exit_code: snapshot.exit_code,
    })
    .unwrap_or_else(|_| "{\"type\":\"status\",\"status\":\"running\"}".to_string());
    let _ = socket.send(WsMessage::Text(status_payload)).await;

    let buffer = session.output_snapshot_tail(snapshot_tail);
    if !buffer.is_empty() {
        let _ = socket.send(WsMessage::Binary(buffer)).await;
    }

    let (mut ws_tx, mut ws_rx) = socket.split();
    let (event_tx, mut event_rx) =
        tokio::sync::mpsc::channel::<WsMessage>(TERMINAL_WS_EVENT_QUEUE_LIMIT);
    let event_tx_output = event_tx.clone();
    let event_tx_status = event_tx.clone();
    let event_tx_input = event_tx.clone();
    let event_tx_ping = event_tx.clone();
    let event_tx_resync = event_tx.clone();
    let needs_tail_resync = Arc::new(AtomicBool::new(false));
    let needs_tail_resync_output = needs_tail_resync.clone();
    let needs_tail_resync_resync = needs_tail_resync.clone();
    let session_output = session.clone();
    let session_resync = session.clone();

    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        while let Some(msg) = event_rx.recv().await {
            if ws_tx.send(msg).await.is_err() {
                break;
            }
        }
    });

    tasks.spawn(async move {
        loop {
            match output_rx.recv().await {
                Ok(bytes) => {
                    if let Some(outcome) = queue_terminal_ws_tail_resync_if_requested(
                        &event_tx_output,
                        &session_output,
                        snapshot_tail,
                        needs_tail_resync_output.as_ref(),
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
                    match queue_terminal_ws_message(&event_tx_output, WsMessage::Binary(bytes)) {
                        TerminalWsQueueOutcome::Enqueued => {}
                        TerminalWsQueueOutcome::Dropped => {
                            request_terminal_ws_tail_resync(needs_tail_resync_output.as_ref());
                            tracing::debug!("dropping terminal output for slow websocket consumer");
                        }
                        TerminalWsQueueOutcome::Closed => break,
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    match queue_terminal_ws_tail_snapshot(
                        &event_tx_output,
                        &session_output,
                        snapshot_tail,
                    ) {
                        TerminalWsQueueOutcome::Enqueued => {}
                        TerminalWsQueueOutcome::Dropped => {
                            request_terminal_ws_tail_resync(needs_tail_resync_output.as_ref());
                            tracing::debug!(
                                "dropping terminal tail resync for slow websocket consumer"
                            );
                        }
                        TerminalWsQueueOutcome::Closed => break,
                    }
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    tasks.spawn(async move {
        loop {
            match status_rx.recv().await {
                Ok(ev) => {
                    let payload = serde_json::to_string(&TerminalServerMessage::Status {
                        status: ev.status,
                        exit_code: ev.exit_code,
                    })
                    .unwrap_or_else(|_| "{\"type\":\"status\",\"status\":\"exited\"}".to_string());
                    match queue_terminal_ws_message(&event_tx_status, WsMessage::Text(payload)) {
                        TerminalWsQueueOutcome::Enqueued => {}
                        TerminalWsQueueOutcome::Dropped => {
                            tracing::debug!("dropping terminal status for slow websocket consumer");
                        }
                        TerminalWsQueueOutcome::Closed => break,
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    request_terminal_ws_tail_resync(needs_tail_resync.as_ref());
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    let session_input = session.clone();
    tasks.spawn(async move {
        while let Some(Ok(msg)) = ws_rx.next().await {
            match msg {
                WsMessage::Binary(data) => {
                    session_input.send_input(data);
                }
                WsMessage::Text(text) => {
                    if let Ok(parsed) = serde_json::from_str::<TerminalClientMessage>(&text) {
                        match parsed {
                            TerminalClientMessage::Resize { cols, rows } => {
                                let _ = session_input.resize(cols, rows);
                            }
                            TerminalClientMessage::Input { data } => {
                                session_input.send_input(data.into_bytes());
                            }
                            TerminalClientMessage::Ping => {
                                let payload = serde_json::to_string(&TerminalServerMessage::Pong)
                                    .unwrap_or_else(|_| "{\"type\":\"pong\"}".to_string());
                                if matches!(
                                    queue_terminal_ws_message(
                                        &event_tx_input,
                                        WsMessage::Text(payload)
                                    ),
                                    TerminalWsQueueOutcome::Closed
                                ) {
                                    break;
                                }
                            }
                        }
                    } else {
                        session_input.send_input(text.into_bytes());
                    }
                }
                WsMessage::Close(_) => break,
                WsMessage::Ping(payload) => {
                    if matches!(
                        queue_terminal_ws_message(&event_tx_input, WsMessage::Pong(payload)),
                        TerminalWsQueueOutcome::Closed
                    ) {
                        break;
                    }
                }
                WsMessage::Pong(_) => {}
            }
        }
    });

    tasks.spawn(async move {
        let mut interval = tokio::time::interval(TERMINAL_PING_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if matches!(
                queue_terminal_ws_message(&event_tx_ping, WsMessage::Ping(Vec::new())),
                TerminalWsQueueOutcome::Closed
            ) {
                break;
            }
        }
    });

    tasks.spawn(async move {
        let mut interval = tokio::time::interval(TERMINAL_TAIL_RESYNC_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            match queue_terminal_ws_tail_resync_if_requested(
                &event_tx_resync,
                &session_resync,
                snapshot_tail,
                needs_tail_resync_resync.as_ref(),
            ) {
                Some(TerminalWsQueueOutcome::Dropped) => {
                    tracing::debug!("dropping terminal tail resync for slow websocket consumer");
                }
                Some(TerminalWsQueueOutcome::Closed) => break,
                Some(TerminalWsQueueOutcome::Enqueued) | None => {}
            }
        }
    });

    let _ = tasks.join_next().await;
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    session.mark_client_disconnected();
}
