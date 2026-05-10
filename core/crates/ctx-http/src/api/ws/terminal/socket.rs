use std::sync::{atomic::AtomicBool, Arc};
use std::time::Duration;

use axum::extract::ws::{Message as WsMessage, WebSocket};
use ctx_transport_runtime::terminals::TerminalSessionHandle;
use futures::StreamExt;
use tokio::task::JoinSet;

mod client;
mod handshake;
mod output;
mod ping;
mod resync;
mod status;
mod writer;

// Terminal bytes are lossy; a slow browser should not force unbounded per-connection buffering.
const TERMINAL_WS_EVENT_QUEUE_LIMIT: usize = 128;
const TERMINAL_PING_INTERVAL: Duration = Duration::from_secs(25);
const TERMINAL_TAIL_RESYNC_INTERVAL: Duration = Duration::from_millis(25);

pub(super) async fn handle_terminal_socket(
    mut socket: WebSocket,
    session: Arc<TerminalSessionHandle>,
    snapshot_tail: usize,
) {
    session.mark_client_connected();
    // Subscribe before capturing the snapshot tail so reconnecting clients do not
    // miss bytes emitted during the status/tail handshake window. This may replay
    // a small overlap from the tail, but it preserves contiguous output delivery.
    let output_rx = session.output_receiver();
    let status_rx = session.status_receiver();

    handshake::send_initial_terminal_snapshot(&mut socket, &session, snapshot_tail).await;

    let (ws_tx, ws_rx) = socket.split();
    let (event_tx, event_rx) =
        tokio::sync::mpsc::channel::<WsMessage>(TERMINAL_WS_EVENT_QUEUE_LIMIT);
    let event_tx_output = event_tx.clone();
    let event_tx_status = event_tx.clone();
    let event_tx_input = event_tx.clone();
    let event_tx_ping = event_tx.clone();
    let event_tx_resync = event_tx.clone();
    let needs_tail_resync = Arc::new(AtomicBool::new(false));
    let needs_tail_resync_output = needs_tail_resync.clone();
    let needs_tail_resync_status = needs_tail_resync.clone();
    let needs_tail_resync_resync = needs_tail_resync.clone();
    let session_output = session.clone();
    let session_input = session.clone();
    let session_resync = session.clone();

    let mut tasks = JoinSet::new();
    tasks.spawn(writer::forward_terminal_ws_messages(ws_tx, event_rx));
    tasks.spawn(output::forward_terminal_output(
        output_rx,
        event_tx_output,
        session_output,
        snapshot_tail,
        needs_tail_resync_output,
    ));
    tasks.spawn(status::forward_terminal_status(
        status_rx,
        event_tx_status,
        needs_tail_resync_status,
    ));
    tasks.spawn(client::handle_terminal_client_messages(
        ws_rx,
        session_input,
        event_tx_input,
    ));
    tasks.spawn(ping::send_terminal_pings(
        event_tx_ping,
        TERMINAL_PING_INTERVAL,
    ));
    tasks.spawn(resync::resync_terminal_tail_when_requested(
        event_tx_resync,
        session_resync,
        snapshot_tail,
        needs_tail_resync_resync,
        TERMINAL_TAIL_RESYNC_INTERVAL,
    ));

    let _ = tasks.join_next().await;
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    session.mark_client_disconnected();
}
