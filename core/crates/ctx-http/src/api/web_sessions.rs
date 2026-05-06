use super::*;
use crate::web_session_launch::{
    WebSessionLaunchError, WebSessionLaunchErrorKind, WebSessionLaunchRequest,
};
use crate::web_sessions::{WebSessionHandle, WebSessionManager};
use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Deserialize)]
pub(super) struct WebSessionCreatePayload {
    session_id: Option<String>,
    worktree_id: Option<String>,
    url: String,
    viewport: Option<WebSessionViewport>,
    fps: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WebSessionStreamAccessQuery {
    pub(crate) token: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct WebSessionListQuery {
    session_id: Option<String>,
}

pub(crate) async fn require_web_session_view_access(
    manager: &Arc<WebSessionManager>,
    id: &str,
    token: Option<&str>,
) -> Result<Arc<WebSessionHandle>, StatusCode> {
    let provided_token = token.ok_or(StatusCode::UNAUTHORIZED)?;
    let handle = manager.get(id).await.ok_or(StatusCode::NOT_FOUND)?;
    if !handle.consume_view_token(provided_token).await {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(handle)
}

pub(crate) async fn require_web_session_signal_access(
    manager: &Arc<WebSessionManager>,
    id: &str,
    token: Option<&str>,
) -> Result<Arc<WebSessionHandle>, StatusCode> {
    let provided_token = token.ok_or(StatusCode::UNAUTHORIZED)?;
    let handle = manager.get(id).await.ok_or(StatusCode::NOT_FOUND)?;
    if !handle.consume_signal_token(provided_token).await {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(handle)
}

async fn map_web_session_action_error(manager: &Arc<WebSessionManager>, id: &str) -> StatusCode {
    if manager.get(id).await.is_none() {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

pub(super) async fn create_web_session(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<WebSessionCreatePayload>,
) -> Result<Json<WebSessionInfo>, (StatusCode, Json<ApiErrorResp>)> {
    if payload.url.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "url is required".to_string(),
            }),
        ));
    }

    let session_id = payload
        .session_id
        .as_deref()
        .map(uuid::Uuid::parse_str)
        .transpose()
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "invalid session id".to_string(),
                }),
            )
        })?
        .map(SessionId);
    let worktree_id = payload
        .worktree_id
        .as_deref()
        .map(uuid::Uuid::parse_str)
        .transpose()
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "invalid worktree id".to_string(),
                }),
            )
        })?
        .map(WorktreeId);

    let info = crate::web_session_launch::create_web_session(
        &state,
        WebSessionLaunchRequest {
            session_id,
            worktree_id,
            url: payload.url,
            viewport: payload.viewport,
            fps: payload.fps,
        },
    )
    .await
    .map_err(web_session_launch_error_response)?;
    Ok(Json(info))
}

fn web_session_launch_error_response(
    error: WebSessionLaunchError,
) -> (StatusCode, Json<ApiErrorResp>) {
    let status = match error.kind() {
        WebSessionLaunchErrorKind::BadRequest => StatusCode::BAD_REQUEST,
        WebSessionLaunchErrorKind::Forbidden => StatusCode::FORBIDDEN,
        WebSessionLaunchErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(ApiErrorResp {
            error: error.message().to_string(),
        }),
    )
}

pub(super) async fn list_web_sessions(
    State(state): State<Arc<AppState>>,
    Query(query): Query<WebSessionListQuery>,
) -> Result<Json<Vec<WebSessionInfo>>, StatusCode> {
    let mut sessions = state.transport.web_sessions.list().await;
    if let Some(session_id) = query.session_id.as_deref() {
        uuid::Uuid::parse_str(session_id).map_err(|_| StatusCode::BAD_REQUEST)?;
        sessions.retain(|session| session.session_id.as_deref() == Some(session_id));
    }
    Ok(Json(sessions))
}

pub(super) async fn get_web_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<WebSessionInfo>, StatusCode> {
    let handle = state
        .transport
        .web_sessions
        .get(&id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(handle.snapshot().await))
}

pub(super) async fn run_web_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(mut payload): Json<WebSessionRunRequest>,
) -> Result<Json<WebSessionRunResponse>, StatusCode> {
    if payload.timeout_ms.is_none() {
        payload.timeout_ms = Some(5 * 60 * 1000);
    }
    let resp = match state.transport.web_sessions.run(&id, payload).await {
        Ok(resp) => resp,
        Err(_) => {
            return Err(map_web_session_action_error(&state.transport.web_sessions, &id).await);
        }
    };
    Ok(Json(resp))
}

pub(super) async fn eval_web_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(mut payload): Json<WebSessionRunRequest>,
) -> Result<Json<WebSessionRunResponse>, StatusCode> {
    if payload.timeout_ms.is_none() {
        payload.timeout_ms = Some(5 * 60 * 1000);
    }
    let resp = match state.transport.web_sessions.eval(&id, payload).await {
        Ok(resp) => resp,
        Err(_) => {
            return Err(map_web_session_action_error(&state.transport.web_sessions, &id).await);
        }
    };
    Ok(Json(resp))
}

pub(super) async fn close_web_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    if state.transport.web_sessions.close(&id).await.is_err() {
        return Err(map_web_session_action_error(&state.transport.web_sessions, &id).await);
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize)]
pub(super) struct WebSessionStreamConnectInfo {
    stream_path: String,
    stream_url: Option<String>,
    expires_at: DateTime<Utc>,
}

pub(super) async fn mint_web_session_stream_token(
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

pub(super) async fn web_session_view(
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
