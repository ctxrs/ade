use axum::extract::ws::{Message as WsMessage, WebSocket};
use ctx_transport_runtime::terminals::{TerminalServerMessage, TerminalSessionHandle};

pub(super) async fn send_initial_terminal_snapshot(
    socket: &mut WebSocket,
    session: &TerminalSessionHandle,
    snapshot_tail: usize,
) {
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
}
