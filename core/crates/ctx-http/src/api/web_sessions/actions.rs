use super::*;
use ctx_transport_runtime::web_sessions::WebSessionManager;

#[derive(Debug, Deserialize, Default)]
pub(in crate::api) struct WebSessionListQuery {
    session_id: Option<String>,
}

async fn map_web_session_action_error(manager: &Arc<WebSessionManager>, id: &str) -> StatusCode {
    if manager.get(id).await.is_none() {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

pub(in crate::api) async fn list_web_sessions(
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

pub(in crate::api) async fn get_web_session(
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

pub(in crate::api) async fn run_web_session(
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

pub(in crate::api) async fn eval_web_session(
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

pub(in crate::api) async fn close_web_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    if state.transport.web_sessions.close(&id).await.is_err() {
        return Err(map_web_session_action_error(&state.transport.web_sessions, &id).await);
    }
    Ok(StatusCode::NO_CONTENT)
}
