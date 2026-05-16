use super::*;

#[path = "workspace_active/send_loop.rs"]
mod send_loop;
#[path = "workspace_active/socket.rs"]
mod socket;

use ctx_daemon::daemon::WorkspaceStreamAccessError;
use socket::handle_workspace_active_snapshot_ws;

fn workspace_stream_access_status(error: WorkspaceStreamAccessError) -> StatusCode {
    match error {
        WorkspaceStreamAccessError::NotFound => StatusCode::NOT_FOUND,
        WorkspaceStreamAccessError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

pub(crate) async fn workspace_active_snapshot_stream_ws(
    ws: WebSocketUpgrade,
    State(state): State<WorkspaceStreamHandle>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let workspace_id = match uuid::Uuid::parse_str(&id) {
        Ok(v) => WorkspaceId(v),
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    if let Err(error) = state
        .require_workspace_active_stream_access(workspace_id)
        .await
    {
        return workspace_stream_access_status(error).into_response();
    }
    ws.on_upgrade(move |socket| handle_workspace_active_snapshot_ws(socket, state, workspace_id))
}
