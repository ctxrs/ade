use super::*;
use ctx_daemon::daemon::TransportHandle;

use ctx_transport_runtime::terminals::DEFAULT_OUTPUT_TAIL_BYTES;

use ctx_daemon::daemon::terminals::TerminalStreamAccessError;

mod queue;
mod socket;

#[cfg(test)]
pub(super) use queue::{
    queue_terminal_ws_message, queue_terminal_ws_tail_resync_if_requested, TerminalWsQueueOutcome,
};

pub(in crate::api) async fn terminal_stream_ws(
    State(state): State<TransportHandle>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    ws: WebSocketUpgrade,
) -> Result<Response, StatusCode> {
    let terminal_id = TerminalId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let session = state
        .require_terminal_stream_access(terminal_id, params.get("token").map(String::as_str))
        .await
        .map_err(terminal_stream_access_status)?;

    let tail_bytes = terminal_stream_tail_bytes(&params);
    Ok(ws.on_upgrade(move |socket| async move {
        socket::handle_terminal_socket(socket, session, tail_bytes).await;
    }))
}

fn terminal_stream_access_status(error: TerminalStreamAccessError) -> StatusCode {
    match error {
        TerminalStreamAccessError::MissingToken | TerminalStreamAccessError::Unauthorized => {
            StatusCode::UNAUTHORIZED
        }
        TerminalStreamAccessError::NotFound => StatusCode::NOT_FOUND,
    }
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
