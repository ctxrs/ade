use super::*;
use crate::daemon::web_sessions::{self, WebSessionActionError};

#[derive(Debug, Deserialize, Default)]
pub(in crate::api) struct WebSessionListQuery {
    session_id: Option<String>,
}

fn web_session_action_status(error: WebSessionActionError) -> StatusCode {
    match error {
        WebSessionActionError::NotFound => StatusCode::NOT_FOUND,
        WebSessionActionError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

pub(in crate::api) async fn list_web_sessions(
    State(state): State<Arc<AppState>>,
    Query(query): Query<WebSessionListQuery>,
) -> Result<Json<Vec<WebSessionInfo>>, StatusCode> {
    let mut sessions = web_sessions::list_web_sessions(&state).await;
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
    let info = web_sessions::get_web_session(&state, &id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(info))
}

pub(in crate::api) async fn run_web_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(mut payload): Json<WebSessionRunRequest>,
) -> Result<Json<WebSessionRunResponse>, StatusCode> {
    if payload.timeout_ms.is_none() {
        payload.timeout_ms = Some(5 * 60 * 1000);
    }
    let resp = web_sessions::run_web_session(&state, &id, payload)
        .await
        .map_err(web_session_action_status)?;
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
    let resp = web_sessions::eval_web_session(&state, &id, payload)
        .await
        .map_err(web_session_action_status)?;
    Ok(Json(resp))
}

pub(in crate::api) async fn close_web_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    web_sessions::close_web_session(&state, &id)
        .await
        .map_err(web_session_action_status)?;
    Ok(StatusCode::NO_CONTENT)
}
