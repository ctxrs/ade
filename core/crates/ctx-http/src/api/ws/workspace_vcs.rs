use std::sync::Arc;

use super::*;

mod buffer;
mod metrics;
mod send_loop;
mod socket;
mod subscription;
#[cfg(test)]
#[path = "workspace_vcs/tests.rs"]
mod tests;

#[cfg(test)]
use self::buffer::VcsPendingBuffer;
#[cfg(test)]
use self::metrics::VcsStreamMetrics;
use self::socket::handle_workspace_vcs_ws;

async fn require_workspace_vcs_stream_access(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
) -> Result<(), StatusCode> {
    let exists = state
        .global_store()
        .get_workspace(workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_some();
    if !exists {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(())
}

pub(crate) async fn workspace_vcs_stream_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let workspace_id = match uuid::Uuid::parse_str(&id) {
        Ok(value) => WorkspaceId(value),
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    if let Err(status) = require_workspace_vcs_stream_access(&state, workspace_id).await {
        return status.into_response();
    }
    ws.on_upgrade(move |socket| handle_workspace_vcs_ws(socket, state, workspace_id))
}
