use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use ctx_core::provider_policy::CODEX_APP_SERVER_ARGS;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::daemon::providers::{accounts, login_sessions, StartedCodexLoginSession};
use crate::daemon::DaemonState;

mod app_server;
mod callback_url;
mod completion;
mod process;

const CODEX_LOGIN_RPC_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub struct CodexLoginStartError {
    message: String,
}

impl CodexLoginStartError {
    fn from_error(err: anyhow::Error) -> Self {
        Self {
            message: ctx_observability::logs::redact_sensitive(&err.to_string()),
        }
    }

    pub fn route_safe_message(&self) -> &str {
        &self.message
    }
}

#[derive(Debug)]
pub struct CodexLoginCompleteResponse {
    pub accepted: bool,
    pub status_code: u16,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum CodexLoginCompleteErrorKind {
    BadRequest,
    NotFound,
    Conflict,
    Unauthorized,
    BadGateway,
    Internal,
}

#[derive(Debug)]
pub struct CodexLoginCompleteError {
    kind: CodexLoginCompleteErrorKind,
    message: String,
}

impl CodexLoginCompleteError {
    fn new(kind: CodexLoginCompleteErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> CodexLoginCompleteErrorKind {
        self.kind
    }

    pub fn route_safe_message(&self) -> &str {
        &self.message
    }
}

pub async fn start_codex_app_server_login(
    state: &Arc<DaemonState>,
    label: Option<String>,
) -> Result<StartedCodexLoginSession, CodexLoginStartError> {
    let prepared = accounts::prepare_codex_login_start(state, label)
        .await
        .map_err(CodexLoginStartError::from_error)?;
    let login = match process::start_codex_login_process(&prepared.account_dir, &prepared.codex_bin)
        .await
    {
        Ok(login) => login,
        Err(err) => {
            let _ = tokio::fs::remove_dir_all(&prepared.account_dir).await;
            return Err(CodexLoginStartError::from_error(err));
        }
    };
    let started_login = login_sessions::start_codex_login_session(
        state,
        prepared.account_id,
        login.auth_url.clone(),
        callback_url::expected_callback_from_auth_url(&login.auth_url),
    )
    .await;

    let state = Arc::clone(state);
    let account_id = started_login.account_id.clone();
    tokio::spawn(async move {
        process::monitor_codex_login(state, account_id, prepared.label, login).await;
    });

    Ok(started_login)
}

pub async fn complete_codex_app_server_login(
    state: &Arc<DaemonState>,
    account_id: &str,
    callback_url: String,
    completion_token: String,
) -> Result<CodexLoginCompleteResponse, CodexLoginCompleteError> {
    let expected_callback =
        login_sessions::claim_codex_login_callback(state, account_id, &completion_token)
            .await
            .map_err(claim_error_response)?;

    if let Err(err) =
        callback_url::validate_callback_url(&callback_url, Some(expected_callback.as_str()))
    {
        login_sessions::restore_codex_login_completion_token(state, account_id, &completion_token)
            .await;
        return Err(CodexLoginCompleteError::new(
            CodexLoginCompleteErrorKind::BadRequest,
            err.to_string(),
        ));
    }

    let status_code = match completion::replay_codex_callback(&callback_url).await {
        Ok(status_code) => status_code,
        Err(err) => {
            if err.should_restore_completion_token() {
                login_sessions::restore_codex_login_completion_token(
                    state,
                    account_id,
                    &completion_token,
                )
                .await;
            }
            return Err(err.into_route_error());
        }
    };

    Ok(CodexLoginCompleteResponse {
        accepted: true,
        status_code,
    })
}

fn claim_error_response(
    err: login_sessions::CodexLoginCallbackClaimError,
) -> CodexLoginCompleteError {
    match err {
        login_sessions::CodexLoginCallbackClaimError::NotFound => {
            CodexLoginCompleteError::new(CodexLoginCompleteErrorKind::NotFound, "login not found")
        }
        login_sessions::CodexLoginCallbackClaimError::NotPending => CodexLoginCompleteError::new(
            CodexLoginCompleteErrorKind::Conflict,
            "login is not pending",
        ),
        login_sessions::CodexLoginCallbackClaimError::InvalidCompletionToken => {
            CodexLoginCompleteError::new(
                CodexLoginCompleteErrorKind::Unauthorized,
                "invalid completion token",
            )
        }
        login_sessions::CodexLoginCallbackClaimError::MissingExpectedCallback => {
            CodexLoginCompleteError::new(
                CodexLoginCompleteErrorKind::Conflict,
                "login is missing expected callback metadata",
            )
        }
    }
}
