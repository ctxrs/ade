use std::sync::Arc;

use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;

use ctx_core::ids::*;

use crate::daemon::AppState;

#[path = "secure_mobile/access.rs"]
mod access;
#[path = "secure_mobile/context.rs"]
mod context;
#[path = "secure_mobile/send_loop.rs"]
mod send_loop;
#[path = "secure_mobile/socket.rs"]
mod socket;

use super::super::MobileSecureStreamQuery;
use access::require_mobile_secure_stream_access;
use socket::handle_mobile_secure_ws;

pub(in crate::api) async fn mobile_secure_workspace_stream_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<MobileSecureStreamQuery>,
) -> impl IntoResponse {
    let workspace_id = match uuid::Uuid::parse_str(&id) {
        Ok(v) => WorkspaceId(v),
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let device_id = query.device_id.trim().to_string();
    let token = query.token.trim().to_string();
    if let Err(status) =
        require_mobile_secure_stream_access(&state, workspace_id, &device_id, &token).await
    {
        return status.into_response();
    }
    ws.on_upgrade(move |socket| async move {
        if let Err(err) = handle_mobile_secure_ws(socket, state, workspace_id, device_id).await {
            tracing::warn!("secure mobile ws ended: {err:#}");
        }
    })
}
