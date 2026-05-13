use ctx_observability::logs;
use ctx_provider_accounts as provider_accounts;

use crate::daemon::AppState;

use super::{new_started_login_session, StartedLoginSession};

pub(crate) async fn start_cursor_login_session(state: &AppState) -> StartedLoginSession {
    let session = new_started_login_session(None, None);
    state
        .providers
        .with_cursor_login_sessions(|map| {
            map.insert(
                session.login_id.clone(),
                provider_accounts::CursorLoginStatus {
                    login_id: session.login_id.clone(),
                    auth_url: session.auth_url.clone(),
                    status: "pending".to_string(),
                    account_id: None,
                    error: None,
                },
            );
        })
        .await;
    session
}

pub(crate) async fn cursor_login_status(
    state: &AppState,
    login_id: &str,
) -> Option<provider_accounts::CursorLoginStatus> {
    state
        .providers
        .with_cursor_login_sessions(|map| map.get(login_id).cloned())
        .await
}

pub(crate) async fn set_cursor_login_error(state: &AppState, login_id: &str, error: String) {
    state
        .providers
        .with_cursor_login_sessions(|map| {
            if let Some(entry) = map.get_mut(login_id) {
                entry.status = "failed".to_string();
                entry.error = Some(error);
            }
        })
        .await;
}

pub(crate) async fn update_cursor_login_auth_url(
    state: &AppState,
    login_id: &str,
    auth_url: String,
) {
    state
        .providers
        .with_cursor_login_sessions(|map| {
            if let Some(entry) = map.get_mut(login_id) {
                entry.auth_url = Some(auth_url);
            }
        })
        .await;
}

pub(crate) async fn start_claude_login_session(
    state: &AppState,
    auth_url: Option<String>,
) -> StartedLoginSession {
    let session = new_started_login_session(auth_url, None);
    state
        .providers
        .with_claude_login_sessions(|map| {
            map.insert(
                session.login_id.clone(),
                provider_accounts::ClaudeLoginStatus {
                    login_id: session.login_id.clone(),
                    auth_url: session.auth_url.clone(),
                    status: "pending".to_string(),
                    account_id: None,
                    error: None,
                },
            );
        })
        .await;
    session
}

pub(crate) async fn claude_login_status(
    state: &AppState,
    login_id: &str,
) -> Option<provider_accounts::ClaudeLoginStatus> {
    state
        .providers
        .with_claude_login_sessions(|map| map.get(login_id).cloned())
        .await
}

pub(crate) async fn set_claude_login_auth_url(state: &AppState, login_id: &str, auth_url: String) {
    state
        .providers
        .with_claude_login_sessions(|map| {
            if let Some(entry) = map.get_mut(login_id) {
                entry.auth_url = Some(auth_url);
            }
        })
        .await;
}

pub(crate) async fn start_kimi_login_session(
    state: &AppState,
    auth_url: Option<String>,
    device_code: Option<String>,
) -> StartedLoginSession {
    let session = new_started_login_session(auth_url, device_code);
    state
        .providers
        .with_kimi_login_sessions(|map| {
            map.insert(
                session.login_id.clone(),
                provider_accounts::KimiLoginStatus {
                    login_id: session.login_id.clone(),
                    status: "pending".to_string(),
                    account_id: None,
                    auth_url: session.auth_url.clone(),
                    device_code: session.device_code.clone(),
                    error: None,
                },
            );
        })
        .await;
    session
}

pub(crate) async fn kimi_login_status(
    state: &AppState,
    login_id: &str,
) -> Option<provider_accounts::KimiLoginStatus> {
    state
        .providers
        .with_kimi_login_sessions(|map| map.get(login_id).cloned())
        .await
}

pub(crate) async fn set_kimi_login_failed(state: &AppState, login_id: &str, error: String) {
    state
        .providers
        .with_kimi_login_sessions(|map| {
            if let Some(entry) = map.get_mut(login_id) {
                entry.status = "failed".to_string();
                entry.error = Some(error);
            }
        })
        .await;
}

pub(crate) async fn set_kimi_login_timeout_if_no_error(
    state: &AppState,
    login_id: &str,
    error: String,
) {
    state
        .providers
        .with_kimi_login_sessions(|map| {
            if let Some(entry) = map.get_mut(login_id) {
                entry.status = "timeout".to_string();
                if entry.error.is_none() {
                    entry.error = Some(error);
                }
            }
        })
        .await;
}

pub(crate) async fn set_kimi_login_terminal_status(
    state: &AppState,
    login_id: &str,
    status: &'static str,
    error: String,
) {
    state
        .providers
        .with_kimi_login_sessions(|map| {
            if let Some(entry) = map.get_mut(login_id) {
                entry.status = status.to_string();
                entry.error = Some(error);
            }
        })
        .await;
}

pub(crate) async fn finish_kimi_login_session(
    state: &AppState,
    login_id: &str,
    account_id: Option<String>,
    restart_error: Option<String>,
) {
    state
        .providers
        .with_kimi_login_sessions(|map| {
            if let Some(entry) = map.get_mut(login_id) {
                entry.account_id = account_id;
                if let Some(error) = restart_error {
                    entry.status = "failed".to_string();
                    entry.error = Some(logs::redact_sensitive(&error));
                } else {
                    entry.status = "success".to_string();
                    entry.error = None;
                }
            }
        })
        .await;
}

pub(crate) async fn finish_cursor_login_session(
    state: &AppState,
    login_id: &str,
    status: String,
    account_id: Option<String>,
    error: Option<String>,
    observed_auth_url: Option<String>,
) {
    state
        .providers
        .with_cursor_login_sessions(|map| {
            if let Some(entry) = map.get_mut(login_id) {
                entry.status = status;
                entry.account_id = account_id;
                entry.error = error;
                if entry.auth_url.is_none() {
                    entry.auth_url = observed_auth_url;
                }
            }
        })
        .await;
}

pub(crate) async fn finish_claude_login_session(
    state: &AppState,
    login_id: &str,
    status: String,
    account_id: Option<String>,
    error: Option<String>,
    observed_auth_url: Option<String>,
) {
    state
        .providers
        .with_claude_login_sessions(|map| {
            if let Some(entry) = map.get_mut(login_id) {
                entry.status = status;
                entry.account_id = account_id;
                entry.error = error;
                if entry.auth_url.is_none() {
                    entry.auth_url = observed_auth_url;
                }
            }
        })
        .await;
}
