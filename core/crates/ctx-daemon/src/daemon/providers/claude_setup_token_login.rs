use std::sync::Arc;

use crate::daemon::providers::{login_sessions, StartedLoginSession};
use crate::daemon::DaemonState;

mod auth_url;
mod runtime;
mod session;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ClaudeSetupTokenLoginStartErrorKind {
    BadRequest,
    Internal,
}

#[derive(Debug)]
pub struct ClaudeSetupTokenLoginStartError {
    kind: ClaudeSetupTokenLoginStartErrorKind,
    message: String,
}

impl ClaudeSetupTokenLoginStartError {
    fn from_runtime_error(err: anyhow::Error) -> Self {
        let message = format!("{err:#}");
        let kind = if message.contains("runtime_command_") {
            ClaudeSetupTokenLoginStartErrorKind::BadRequest
        } else {
            ClaudeSetupTokenLoginStartErrorKind::Internal
        };
        Self::from_message(kind, message)
    }

    fn from_internal_error(err: anyhow::Error) -> Self {
        Self::new(ClaudeSetupTokenLoginStartErrorKind::Internal, err)
    }

    fn new(kind: ClaudeSetupTokenLoginStartErrorKind, err: anyhow::Error) -> Self {
        Self::from_message(kind, format!("{err:#}"))
    }

    fn from_message(kind: ClaudeSetupTokenLoginStartErrorKind, message: String) -> Self {
        Self {
            kind,
            message: ctx_observability::logs::redact_sensitive(&message),
        }
    }

    pub fn kind(&self) -> ClaudeSetupTokenLoginStartErrorKind {
        self.kind
    }

    pub fn route_safe_message(&self) -> &str {
        &self.message
    }
}

pub async fn start_claude_setup_token_login(
    state: &Arc<DaemonState>,
    label: Option<String>,
) -> Result<StartedLoginSession, ClaudeSetupTokenLoginStartError> {
    let runtime = super::login_runtime::resolve_claude_login_runtime(state)
        .await
        .map_err(ClaudeSetupTokenLoginStartError::from_runtime_error)?;
    let login = session::start_claude_login_process(&runtime)
        .await
        .map_err(ClaudeSetupTokenLoginStartError::from_internal_error)?;
    let auth_url = login.auth_url.clone();
    let login_session = login_sessions::start_claude_login_session(state, auth_url).await;

    let state = Arc::clone(state);
    let login_id = login_session.login_id.clone();
    tokio::spawn(async move {
        session::monitor_claude_login(state, login_id, label, login).await;
    });

    Ok(login_session)
}
