use super::*;

#[path = "workspace_active/send_loop.rs"]
mod send_loop;
#[path = "workspace_active/socket.rs"]
mod socket;

use socket::handle_workspace_active_snapshot_ws;

async fn require_workspace_active_stream_access(
    state: &WorkspaceStreamHandle,
    workspace_id: WorkspaceId,
) -> Result<(), StatusCode> {
    let exists = state
        .workspace_exists(workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !exists {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(())
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
    if let Err(status) = require_workspace_active_stream_access(&state, workspace_id).await {
        return status.into_response();
    }
    ws.on_upgrade(move |socket| handle_workspace_active_snapshot_ws(socket, state, workspace_id))
}
