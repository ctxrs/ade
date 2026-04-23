use super::*;
use crate::web_session_launch::WebSessionLaunchRequest;
use crate::web_sessions::{WebSessionHandle, WebSessionManager};

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

pub(crate) async fn require_web_session_stream_access(
    manager: &Arc<WebSessionManager>,
    id: &str,
    token: Option<&str>,
) -> Result<Arc<WebSessionHandle>, StatusCode> {
    let provided_token = token.ok_or(StatusCode::UNAUTHORIZED)?;
    let handle = manager.get(id).await.ok_or(StatusCode::NOT_FOUND)?;
    if !handle.matches_stream_token(provided_token) {
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
    headers: HeaderMap,
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

    let mut info = crate::web_session_launch::create_web_session(
        &state,
        WebSessionLaunchRequest {
            session_id,
            worktree_id,
            url: payload.url,
            viewport: payload.viewport,
            fps: payload.fps,
        },
    )
    .await?;
    let base_url = resolve_request_base_url(&headers, &state.core.daemon_url);
    info.stream_url = Some(format!("{}{}", base_url, info.stream_path));
    Ok(Json(info))
}

pub(super) async fn list_web_sessions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<WebSessionInfo>>, StatusCode> {
    let mut sessions = state.transport.web_sessions.list().await;
    let base_url = resolve_request_base_url(&headers, &state.core.daemon_url);
    for session in sessions.iter_mut() {
        session.stream_url = Some(format!("{}{}", base_url, session.stream_path));
    }
    Ok(Json(sessions))
}

pub(super) async fn get_web_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<WebSessionInfo>, StatusCode> {
    let handle = state
        .transport
        .web_sessions
        .get(&id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    let mut info = handle.snapshot().await;
    let base_url = resolve_request_base_url(&headers, &state.core.daemon_url);
    info.stream_url = Some(format!("{}{}", base_url, info.stream_path));
    Ok(Json(info))
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

pub(super) async fn web_session_view(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<WebSessionStreamAccessQuery>,
) -> Result<Response, StatusCode> {
    let handle = require_web_session_stream_access(
        &state.transport.web_sessions,
        &id,
        query.token.as_deref(),
    )
    .await?;
    let info = handle.snapshot().await;
    let body = render_web_session_view(&info, &handle.signal_path());
    Ok(([(header::CONTENT_TYPE, "text/html; charset=utf-8")], body).into_response())
}
