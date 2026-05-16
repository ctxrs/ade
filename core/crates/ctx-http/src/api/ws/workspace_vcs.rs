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
use ctx_daemon::daemon::WorkspaceStreamAccessError;

fn workspace_stream_access_status(error: WorkspaceStreamAccessError) -> StatusCode {
    match error {
        WorkspaceStreamAccessError::NotFound => StatusCode::NOT_FOUND,
        WorkspaceStreamAccessError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

pub(crate) async fn workspace_vcs_stream_ws(
    ws: WebSocketUpgrade,
    State(state): State<WorkspacesHandle>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let workspace_id = match uuid::Uuid::parse_str(&id) {
        Ok(value) => WorkspaceId(value),
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    if let Err(error) = state
        .require_workspace_vcs_stream_access(workspace_id)
        .await
    {
        return workspace_stream_access_status(error).into_response();
    }
    ws.on_upgrade(move |socket| handle_workspace_vcs_ws(socket, state, workspace_id))
}
