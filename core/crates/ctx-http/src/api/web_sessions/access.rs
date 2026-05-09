use super::*;
use ctx_transport_runtime::web_sessions::{WebSessionHandle, WebSessionManager};

#[derive(Debug, Deserialize)]
pub(crate) struct WebSessionStreamAccessQuery {
    pub(crate) token: Option<String>,
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
