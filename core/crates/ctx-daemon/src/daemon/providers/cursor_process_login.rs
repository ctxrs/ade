use std::path::{Path as StdPath, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use ctx_observability::logs;
use serde::Deserialize;

use crate::daemon::providers::{login_runtime, login_sessions, StartedLoginSession};
use crate::daemon::DaemonState;
#[cfg(test)]
use login_runtime::resolve_cursor_login_runtime_from_config;

mod auth_url;
mod capture;
mod output;
mod session;
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorProcessLoginStartErrorKind {
    RuntimeCommandBadRequest,
    InternalStartup,
}

#[derive(Debug)]
pub struct CursorProcessLoginStartError {
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

    pub fn kind(&self) -> CursorProcessLoginStartErrorKind {
        self.kind
    }

    pub fn route_safe_message(&self) -> &str {
        &self.message
    }
}

pub async fn start_cursor_process_login(
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
