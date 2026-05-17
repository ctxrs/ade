use std::path::{Path as StdPath, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use ctx_observability::logs;
use ctx_provider_accounts as provider_accounts;
use serde::Deserialize;
use serde::Serialize;

use crate::daemon::providers::{login_runtime, login_sessions, StartedLoginSession};
use crate::daemon::{DaemonState, ProvidersHandle};
#[cfg(test)]
use login_runtime::resolve_cursor_login_runtime_from_config;

mod auth_url;
mod capture;
mod output;
mod session;
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CursorProcessLoginStartErrorKind {
    RuntimeCommandBadRequest,
    InternalStartup,
}

#[derive(Debug)]
struct CursorProcessLoginStartError {
    kind: CursorProcessLoginStartErrorKind,
    message: String,
}

impl CursorProcessLoginStartError {
    fn from_runtime_error(err: anyhow::Error) -> Self {
        let raw_message = err.to_string();
        let kind = if raw_message.contains("runtime_command_") {
            CursorProcessLoginStartErrorKind::RuntimeCommandBadRequest
        } else {
            CursorProcessLoginStartErrorKind::InternalStartup
        };
        let message = logs::redact_sensitive(&raw_message);
        Self { kind, message }
    }

    fn kind(&self) -> CursorProcessLoginStartErrorKind {
        self.kind
    }

    fn route_safe_message(&self) -> &str {
        &self.message
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct CursorLoginStartRouteRequest {
    label: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CursorLoginStartRouteResponse {
    login_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_url: Option<String>,
}

impl From<StartedLoginSession> for CursorLoginStartRouteResponse {
    fn from(session: StartedLoginSession) -> Self {
        Self {
            login_id: session.login_id,
            auth_url: session.auth_url,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorLoginRouteErrorKind {
    BadRequest,
    NotFound,
    Internal,
}

#[derive(Debug)]
pub struct CursorLoginRouteError {
    kind: CursorLoginRouteErrorKind,
    message: String,
}

impl CursorLoginRouteError {
    pub fn kind(&self) -> CursorLoginRouteErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    fn new(kind: CursorLoginRouteErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl ProvidersHandle {
    pub async fn start_cursor_login_for_route(
        &self,
        request: CursorLoginStartRouteRequest,
    ) -> Result<CursorLoginStartRouteResponse, CursorLoginRouteError> {
        start_cursor_process_login(&self.state, request.label)
            .await
            .map(CursorLoginStartRouteResponse::from)
            .map_err(cursor_login_start_route_error)
    }

    pub async fn cursor_login_status_for_route(
        &self,
        login_id: &str,
    ) -> Result<provider_accounts::CursorLoginStatus, CursorLoginRouteError> {
        login_sessions::cursor_login_status(&self.state, login_id)
            .await
            .ok_or_else(cursor_login_not_found_route_error)
    }
}

fn cursor_login_not_found_route_error() -> CursorLoginRouteError {
    CursorLoginRouteError::new(CursorLoginRouteErrorKind::NotFound, "login not found")
}

fn cursor_login_start_route_error(error: CursorProcessLoginStartError) -> CursorLoginRouteError {
    let kind = match error.kind() {
        CursorProcessLoginStartErrorKind::RuntimeCommandBadRequest => {
            CursorLoginRouteErrorKind::BadRequest
        }
        CursorProcessLoginStartErrorKind::InternalStartup => CursorLoginRouteErrorKind::Internal,
    };
    CursorLoginRouteError::new(kind, error.route_safe_message().to_string())
}

async fn start_cursor_process_login(
    state: &Arc<DaemonState>,
    label: Option<String>,
) -> Result<StartedLoginSession, CursorProcessLoginStartError> {
    let cursor_runtime = login_runtime::resolve_cursor_login_runtime(state)
        .await
        .map_err(CursorProcessLoginStartError::from_runtime_error)?;
    let login_session = login_sessions::start_cursor_login_session(state).await;

    let state = Arc::clone(state);
    let login_id = login_session.login_id.clone();
    tokio::spawn(async move {
        session::monitor_cursor_login(state, cursor_runtime, login_id, label).await;
    });

    Ok(login_session)
}

#[cfg(test)]
mod route_tests {
    use super::*;

    #[test]
    fn cursor_login_route_missing_status_preserves_not_found_message() {
        let error = cursor_login_not_found_route_error();

        assert_eq!(error.kind(), CursorLoginRouteErrorKind::NotFound);
        assert_eq!(error.message(), "login not found");
    }

    #[test]
    fn cursor_login_route_start_error_maps_status_classes() {
        let cases = [
            (
                CursorProcessLoginStartErrorKind::RuntimeCommandBadRequest,
                CursorLoginRouteErrorKind::BadRequest,
            ),
            (
                CursorProcessLoginStartErrorKind::InternalStartup,
                CursorLoginRouteErrorKind::Internal,
            ),
        ];

        for (source, expected) in cases {
            let error = cursor_login_start_route_error(CursorProcessLoginStartError {
                kind: source,
                message: "boom".to_string(),
            });
            assert_eq!(error.kind(), expected);
            assert_eq!(error.message(), "boom");
        }
    }

    #[test]
    fn cursor_login_route_start_response_omits_absent_auth_url() {
        let payload =
            serde_json::to_value(CursorLoginStartRouteResponse::from(StartedLoginSession {
                login_id: "login-1".to_string(),
                auth_url: None,
                device_code: None,
            }))
            .unwrap();

        assert_eq!(payload["login_id"].as_str(), Some("login-1"));
        assert!(payload.get("auth_url").is_none());
    }

    #[test]
    fn cursor_login_route_start_response_preserves_auth_url() {
        let payload =
            serde_json::to_value(CursorLoginStartRouteResponse::from(StartedLoginSession {
                login_id: "login-2".to_string(),
                auth_url: Some("https://cursor.com/login/device?code=test".to_string()),
                device_code: None,
            }))
            .unwrap();

        assert_eq!(
            payload["auth_url"].as_str(),
            Some("https://cursor.com/login/device?code=test")
        );
    }
}
