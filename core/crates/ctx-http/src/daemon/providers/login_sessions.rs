use ctx_observability::logs;
use ctx_provider_accounts as provider_accounts;

use crate::daemon::AppState;

#[derive(Debug)]
pub(crate) struct StartedLoginSession {
    pub(crate) login_id: String,
    pub(crate) auth_url: Option<String>,
    pub(crate) device_code: Option<String>,
}

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

fn new_started_login_session(
    auth_url: Option<String>,
    device_code: Option<String>,
) -> StartedLoginSession {
    StartedLoginSession {
        login_id: uuid::Uuid::new_v4().to_string(),
        auth_url,
        device_code,
    }
}

fn restart_failure_message(error: anyhow::Error) -> String {
    logs::redact_sensitive(&format!(
        "auth saved but provider restart failed: {error:#}"
    ))
}

macro_rules! auth_account_login_session_helpers {
    (
        $start:ident,
        $status:ident,
        $set_failed:ident,
        $set_failed_if_no_error:ident,
        $set_timeout_if_no_error:ident,
        $set_auth_url:ident,
        $with:ident,
        $status_ty:ty
    ) => {
        pub(crate) async fn $start(state: &AppState) -> StartedLoginSession {
            type LoginStatus = $status_ty;
            let session = new_started_login_session(None, None);
            state
                .providers
                .$with(|map| {
                    map.insert(
                        session.login_id.clone(),
                        LoginStatus {
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

        pub(crate) async fn $status(state: &AppState, login_id: &str) -> Option<$status_ty> {
            state
                .providers
                .$with(|map| map.get(login_id).cloned())
                .await
        }

        pub(crate) async fn $set_failed(state: &AppState, login_id: &str, error: String) {
            state
                .providers
                .$with(|map| {
                    if let Some(entry) = map.get_mut(login_id) {
                        entry.status = "failed".to_string();
                        entry.error = Some(error);
                    }
                })
                .await;
        }

        pub(crate) async fn $set_failed_if_no_error(
            state: &AppState,
            login_id: &str,
            error: String,
        ) {
            state
                .providers
                .$with(|map| {
                    if let Some(entry) = map.get_mut(login_id) {
                        entry.status = "failed".to_string();
                        if entry.error.is_none() {
                            entry.error = Some(error);
                        }
                    }
                })
                .await;
        }

        pub(crate) async fn $set_timeout_if_no_error(
            state: &AppState,
            login_id: &str,
            error: String,
        ) {
            state
                .providers
                .$with(|map| {
                    if let Some(entry) = map.get_mut(login_id) {
                        entry.status = "timeout".to_string();
                        if entry.error.is_none() {
                            entry.error = Some(error);
                        }
                    }
                })
                .await;
        }

        pub(crate) async fn $set_auth_url(state: &AppState, login_id: &str, auth_url: String) {
            state
                .providers
                .$with(|map| {
                    if let Some(entry) = map.get_mut(login_id) {
                        entry.auth_url = Some(auth_url);
                    }
                })
                .await;
        }
    };
}

macro_rules! auth_only_login_session_helpers {
    (
        $start:ident,
        $status:ident,
        $set_failed:ident,
        $set_failed_if_no_error:ident,
        $set_timeout_if_no_error:ident,
        $set_auth_url:ident,
        $with:ident,
        $status_ty:ty
    ) => {
        pub(crate) async fn $start(state: &AppState) -> StartedLoginSession {
            type LoginStatus = $status_ty;
            let session = new_started_login_session(None, None);
            state
                .providers
                .$with(|map| {
                    map.insert(
                        session.login_id.clone(),
                        LoginStatus {
                            login_id: session.login_id.clone(),
                            auth_url: session.auth_url.clone(),
                            status: "pending".to_string(),
                            error: None,
                        },
                    );
                })
                .await;
            session
        }

        pub(crate) async fn $status(state: &AppState, login_id: &str) -> Option<$status_ty> {
            state
                .providers
                .$with(|map| map.get(login_id).cloned())
                .await
        }

        pub(crate) async fn $set_failed(state: &AppState, login_id: &str, error: String) {
            state
                .providers
                .$with(|map| {
                    if let Some(entry) = map.get_mut(login_id) {
                        entry.status = "failed".to_string();
                        entry.error = Some(error);
                    }
                })
                .await;
        }

        pub(crate) async fn $set_failed_if_no_error(
            state: &AppState,
            login_id: &str,
            error: String,
        ) {
            state
                .providers
                .$with(|map| {
                    if let Some(entry) = map.get_mut(login_id) {
                        entry.status = "failed".to_string();
                        if entry.error.is_none() {
                            entry.error = Some(error);
                        }
                    }
                })
                .await;
        }

        pub(crate) async fn $set_timeout_if_no_error(
            state: &AppState,
            login_id: &str,
            error: String,
        ) {
            state
                .providers
                .$with(|map| {
                    if let Some(entry) = map.get_mut(login_id) {
                        entry.status = "timeout".to_string();
                        if entry.error.is_none() {
                            entry.error = Some(error);
                        }
                    }
                })
                .await;
        }

        pub(crate) async fn $set_auth_url(state: &AppState, login_id: &str, auth_url: String) {
            state
                .providers
                .$with(|map| {
                    if let Some(entry) = map.get_mut(login_id) {
                        entry.auth_url = Some(auth_url);
                    }
                })
                .await;
        }
    };
}

auth_account_login_session_helpers!(
    start_gemini_login_session,
    gemini_login_status,
    set_gemini_login_failed,
    set_gemini_login_failed_if_no_error,
    set_gemini_login_timeout_if_no_error,
    set_gemini_login_auth_url,
    with_gemini_login_sessions,
    provider_accounts::GeminiLoginStatus
);

auth_account_login_session_helpers!(
    start_qwen_login_session,
    qwen_login_status,
    set_qwen_login_failed,
    set_qwen_login_failed_if_no_error,
    set_qwen_login_timeout_if_no_error,
    set_qwen_login_auth_url,
    with_qwen_login_sessions,
    provider_accounts::QwenLoginStatus
);

auth_only_login_session_helpers!(
    start_amp_login_session,
    amp_login_status,
    set_amp_login_failed,
    set_amp_login_failed_if_no_error,
    set_amp_login_timeout_if_no_error,
    set_amp_login_auth_url,
    with_amp_login_sessions,
    provider_accounts::AmpLoginStatus
);

auth_only_login_session_helpers!(
    start_mistral_login_session,
    mistral_login_status,
    set_mistral_login_failed,
    set_mistral_login_failed_if_no_error,
    set_mistral_login_timeout_if_no_error,
    set_mistral_login_auth_url,
    with_mistral_login_sessions,
    provider_accounts::MistralLoginStatus
);

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

pub(crate) async fn finish_gemini_login_session(
    state: &AppState,
    login_id: &str,
    account_id: Option<String>,
    restart_error: Option<String>,
) {
    state
        .providers
        .with_gemini_login_sessions(|map| {
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

pub(crate) async fn finish_qwen_login_session(
    state: &AppState,
    login_id: &str,
    account_id: Option<String>,
    restart_result: anyhow::Result<()>,
) {
    state
        .providers
        .with_qwen_login_sessions(|map| {
            if let Some(entry) = map.get_mut(login_id) {
                entry.account_id = account_id;
                match restart_result {
                    Ok(()) => {
                        entry.status = "success".to_string();
                        entry.error = None;
                    }
                    Err(err) => {
                        entry.status = "failed".to_string();
                        entry.error = Some(restart_failure_message(err));
                    }
                }
            }
        })
        .await;
}

pub(crate) async fn finish_amp_login_session(
    state: &AppState,
    login_id: &str,
    restart_result: anyhow::Result<()>,
) {
    state
        .providers
        .with_amp_login_sessions(|map| {
            if let Some(entry) = map.get_mut(login_id) {
                entry.auth_url = None;
                match restart_result {
                    Ok(()) => {
                        entry.status = "success".to_string();
                        entry.error = None;
                    }
                    Err(err) => {
                        entry.status = "failed".to_string();
                        entry.error = Some(restart_failure_message(err));
                    }
                }
            }
        })
        .await;
}

pub(crate) async fn finish_mistral_login_session(
    state: &AppState,
    login_id: &str,
    restart_result: anyhow::Result<()>,
) {
    state
        .providers
        .with_mistral_login_sessions(|map| {
            if let Some(entry) = map.get_mut(login_id) {
                entry.auth_url = None;
                match restart_result {
                    Ok(()) => {
                        entry.status = "success".to_string();
                        entry.error = None;
                    }
                    Err(err) => {
                        entry.status = "failed".to_string();
                        entry.error = Some(restart_failure_message(err));
                    }
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

pub(crate) async fn codex_login_statuses(
    state: &AppState,
) -> Vec<provider_accounts::CodexLoginStatus> {
    state
        .providers
        .with_codex_login_sessions(|map| map.values().cloned().collect())
        .await
}

pub(crate) async fn remove_codex_login_session(
    state: &AppState,
    account_id: &str,
) -> Vec<provider_accounts::CodexLoginStatus> {
    state
        .providers
        .with_codex_login_sessions(|map| {
            map.remove(account_id);
            map.values().cloned().collect()
        })
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
