use super::access::WebSessionStreamAccessQuery;
use super::*;
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::daemon::web_sessions::{self as daemon_web_sessions, WebSessionAccessError};

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
    let connect_path = daemon_web_sessions::mint_web_session_view_connect_path(&state, &id)
        .await
        .map_err(web_session_access_status)?;
    let stream_url = match state.core.public_base_url.as_deref() {
        Some(base_url) => Some(
            public_route_url(base_url, &connect_path.stream_path)
                .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?,
        ),
        None => resolve_request_base_url(&headers, &state.core.daemon_url, None)
            .map(|base_url| format!("{base_url}{}", connect_path.stream_path)),
    };
    Ok(Json(WebSessionStreamConnectInfo {
        stream_path: connect_path.stream_path,
        stream_url,
        expires_at: connect_path.expires_at,
    }))
}

pub(in crate::api) async fn web_session_view(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<WebSessionStreamAccessQuery>,
) -> Result<Response, StatusCode> {
    let view_page =
        daemon_web_sessions::prepare_web_session_view_page(&state, &id, query.token.as_deref())
            .await
            .map_err(web_session_access_status)?;
    let signal_endpoint = match state.core.public_base_url.as_deref() {
        Some(base_url) => public_websocket_url(base_url, &view_page.signal_path)
            .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?,
        None => view_page.signal_path,
    };
    let body = render_web_session_view(&view_page.info, &signal_endpoint);
    Ok(([(header::CONTENT_TYPE, "text/html; charset=utf-8")], body).into_response())
}

pub(crate) fn web_session_access_status(error: WebSessionAccessError) -> StatusCode {
    match error {
        WebSessionAccessError::MissingToken | WebSessionAccessError::Unauthorized => {
            StatusCode::UNAUTHORIZED
        }
        WebSessionAccessError::NotFound => StatusCode::NOT_FOUND,
    }
}
