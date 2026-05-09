use super::*;
use std::sync::Arc;

use ctx_transport_runtime::terminals::{
    TerminalManager, TerminalSessionHandle, DEFAULT_OUTPUT_TAIL_BYTES,
};

mod queue;
mod socket;

#[cfg(test)]
pub(super) use queue::{
    queue_terminal_ws_message, queue_terminal_ws_tail_resync_if_requested, TerminalWsQueueOutcome,
};

async fn require_terminal_stream_access(
    manager: &Arc<TerminalManager>,
    id: TerminalId,
    token: Option<&str>,
) -> Result<Arc<TerminalSessionHandle>, StatusCode> {
    let provided_token = token.ok_or(StatusCode::UNAUTHORIZED)?;
    let handle = manager.get(id).await.ok_or(StatusCode::NOT_FOUND)?;
    if !handle.consume_stream_token(provided_token) {
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
        socket::handle_terminal_socket(socket, session, tail_bytes).await;
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
        .unwrap_or(DEFAULT_OUTPUT_TAIL_BYTES)
}
