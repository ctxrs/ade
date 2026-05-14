use std::sync::Arc;

use chrono::{DateTime, Utc};
use ctx_transport_runtime::web_sessions::{WebSessionHandle, WebSessionInfo};

use crate::daemon::DaemonState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebSessionAccessError {
    MissingToken,
    NotFound,
    Unauthorized,
}

pub struct WebSessionViewConnectPath {
    pub stream_path: String,
    pub expires_at: DateTime<Utc>,
}

pub struct WebSessionViewPage {
    pub info: WebSessionInfo,
    pub signal_path: String,
}

pub async fn mint_web_session_view_connect_path(
    state: &Arc<DaemonState>,
    id: &str,
) -> Result<WebSessionViewConnectPath, WebSessionAccessError> {
    let handle = state
        .transport
        .web_sessions
        .get(id)
        .await
        .ok_or(WebSessionAccessError::NotFound)?;
    let (stream_path, expires_at) = handle.issue_view_connect_path().await;
    Ok(WebSessionViewConnectPath {
        stream_path,
        expires_at,
    })
}

pub async fn prepare_web_session_view_page(
    state: &Arc<DaemonState>,
    id: &str,
    token: Option<&str>,
) -> Result<WebSessionViewPage, WebSessionAccessError> {
    let handle = require_web_session_view_access(state, id, token).await?;
    let info = handle.snapshot().await;
    let (signal_path, _) = handle.issue_signal_connect_path().await;
    Ok(WebSessionViewPage { info, signal_path })
}

pub async fn authorize_web_session_signal_access(
    state: &Arc<DaemonState>,
    id: &str,
    token: Option<&str>,
) -> Result<(), WebSessionAccessError> {
    require_web_session_signal_access(state, id, token)
        .await
        .map(|_| ())
}

async fn require_web_session_view_access(
    state: &Arc<DaemonState>,
    id: &str,
    token: Option<&str>,
) -> Result<Arc<WebSessionHandle>, WebSessionAccessError> {
    let provided_token = token.ok_or(WebSessionAccessError::MissingToken)?;
    let handle = state
        .transport
        .web_sessions
        .get(id)
        .await
        .ok_or(WebSessionAccessError::NotFound)?;
    if !handle.consume_view_token(provided_token).await {
        return Err(WebSessionAccessError::Unauthorized);
    }
    Ok(handle)
}

async fn require_web_session_signal_access(
    state: &Arc<DaemonState>,
    id: &str,
    token: Option<&str>,
) -> Result<Arc<WebSessionHandle>, WebSessionAccessError> {
    let provided_token = token.ok_or(WebSessionAccessError::MissingToken)?;
    let handle = state
        .transport
        .web_sessions
        .get(id)
        .await
        .ok_or(WebSessionAccessError::NotFound)?;
    if !handle.consume_signal_token(provided_token).await {
        return Err(WebSessionAccessError::Unauthorized);
    }
    Ok(handle)
}
