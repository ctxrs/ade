use ctx_provider_accounts as provider_accounts;

use crate::daemon::AppState;

#[derive(Debug)]
pub(crate) struct StartedCodexLoginSession {
    pub(crate) account_id: String,
    pub(crate) auth_url: String,
    pub(crate) expected_callback_url: Option<String>,
    pub(crate) completion_token: String,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum CodexLoginCallbackClaimError {
    NotFound,
    NotPending,
    InvalidCompletionToken,
    MissingExpectedCallback,
}

pub(crate) async fn start_codex_login_session(
    state: &AppState,
    account_id: String,
    auth_url: String,
    expected_callback_url: Option<String>,
) -> StartedCodexLoginSession {
    let completion_token = uuid::Uuid::new_v4().to_string();
    let status = provider_accounts::CodexLoginStatus {
        account_id: account_id.clone(),
        auth_url: auth_url.clone(),
        expected_callback_url: expected_callback_url.clone(),
        completion_token: Some(completion_token.clone()),
        status: "pending".to_string(),
        error: None,
    };
    state
        .providers
        .with_codex_login_sessions(|map| {
            map.insert(account_id.clone(), status);
        })
        .await;

    StartedCodexLoginSession {
        account_id,
        auth_url,
        expected_callback_url,
        completion_token,
    }
}

pub(crate) async fn codex_login_status(
    state: &AppState,
    account_id: &str,
) -> Option<provider_accounts::CodexLoginStatus> {
    state
        .providers
        .with_codex_login_sessions(|map| map.get(account_id).cloned())
        .await
}

pub(crate) async fn claim_codex_login_callback(
    state: &AppState,
    account_id: &str,
    completion_token: &str,
) -> Result<String, CodexLoginCallbackClaimError> {
    state
        .providers
        .with_codex_login_sessions(|map| {
            let Some(status) = map.get_mut(account_id) else {
                return Err(CodexLoginCallbackClaimError::NotFound);
            };
            if status.status != "pending" {
                return Err(CodexLoginCallbackClaimError::NotPending);
            }
            if status.completion_token.as_deref() != Some(completion_token) {
                return Err(CodexLoginCallbackClaimError::InvalidCompletionToken);
            }
            let Some(expected_callback) = status.expected_callback_url.clone() else {
                return Err(CodexLoginCallbackClaimError::MissingExpectedCallback);
            };
            status.completion_token = None;
            Ok(expected_callback)
        })
        .await
}

pub(crate) async fn restore_codex_login_completion_token(
    state: &AppState,
    account_id: &str,
    completion_token: &str,
) {
    state
        .providers
        .with_codex_login_sessions(|map| {
            if let Some(status) = map.get_mut(account_id) {
                if status.status == "pending" && status.completion_token.is_none() {
                    status.completion_token = Some(completion_token.to_string());
                }
            }
        })
        .await;
}

pub(crate) async fn finish_codex_login_session(
    state: &AppState,
    account_id: &str,
    success: bool,
    error: Option<String>,
) {
    state
        .providers
        .with_codex_login_sessions(|map| {
            if let Some(entry) = map.get_mut(account_id) {
                entry.status = if success {
                    "success".to_string()
                } else {
                    "failed".to_string()
                };
                entry.completion_token = None;
                entry.error = error;
            }
        })
        .await;
}
