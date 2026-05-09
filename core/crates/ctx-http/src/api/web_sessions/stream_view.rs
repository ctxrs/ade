use super::access::{require_web_session_view_access, WebSessionStreamAccessQuery};
use super::*;
use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(in crate::api) struct WebSessionStreamConnectInfo {
    stream_path: String,
    stream_url: Option<String>,
    expires_at: DateTime<Utc>,
}

pub(in crate::api) async fn mint_web_session_stream_token(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<WebSessionStreamConnectInfo>, StatusCode> {
    let handle = state
        .transport
        .web_sessions
        .get(&id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    let (stream_path, expires_at) = handle.issue_view_connect_path().await;
    let stream_url = match state.core.public_base_url.as_deref() {
        Some(base_url) => Some(
            public_route_url(base_url, &stream_path).ok_or(StatusCode::INTERNAL_SERVER_ERROR)?,
        ),
        None => resolve_request_base_url(&headers, &state.core.daemon_url, None)
            .map(|base_url| format!("{base_url}{stream_path}")),
    };
    Ok(Json(WebSessionStreamConnectInfo {
        stream_path,
        stream_url,
        expires_at,
    }))
}

pub(in crate::api) async fn web_session_view(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<WebSessionStreamAccessQuery>,
) -> Result<Response, StatusCode> {
    let handle =
        require_web_session_view_access(&state.transport.web_sessions, &id, query.token.as_deref())
            .await?;
    let info = handle.snapshot().await;
    let (signal_path, _) = handle.issue_signal_connect_path().await;
    let signal_endpoint = match state.core.public_base_url.as_deref() {
        Some(base_url) => {
            public_websocket_url(base_url, &signal_path).ok_or(StatusCode::INTERNAL_SERVER_ERROR)?
        }
        None => signal_path,
    };
    let body = render_web_session_view(&info, &signal_endpoint);
    Ok(([(header::CONTENT_TYPE, "text/html; charset=utf-8")], body).into_response())
}
