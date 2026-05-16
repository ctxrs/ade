use std::sync::Arc;
use std::time::Instant;

use ctx_core::models::SessionEventType;
use ctx_observability::logs;
use tokio::sync::mpsc;

use crate::daemon::providers::{accounts, login_sessions, StartedLoginSession};
use crate::daemon::DaemonState;

use super::common;

pub async fn start_mistral_browser_login(
    state: &Arc<DaemonState>,
    label: Option<String>,
) -> StartedLoginSession {
    let session = login_sessions::start_mistral_login_session(state).await;
    let login_id = session.login_id.clone();
    let state = Arc::clone(state);
    tokio::spawn(async move {
        monitor_mistral_login(state, login_id, label).await;
    });
    session
}

async fn monitor_mistral_login(state: Arc<DaemonState>, login_id: String, label: Option<String>) {
    let paths = match accounts::prepare_mistral_login_paths(&state, &login_id).await {
        Ok(paths) => paths,
        Err(error) => {
            login_sessions::set_mistral_login_failed(&state, &login_id, error).await;
            return;
        }
    };
    let provider_env = accounts::mistral_login_provider_env(&state, &paths.mistral_home);
    let mut event_rx = match common::authenticate_browser_login_session(
        &state,
        "mistral",
        format!("mistral-login-{login_id}"),
        paths.workdir.clone(),
        provider_env,
        None,
    )
    .await
    {
        Ok(event_rx) => event_rx,
        Err(err) => {
            login_sessions::set_mistral_login_failed(&state, &login_id, err).await;
            common::cleanup_login_home(&paths.login_home).await;
            return;
        }
    };

    let started_at = Instant::now();
    let timeout = common::mistral_login_timeout();
    let mut progress = MistralLoginProgress::default();

    loop {
        if started_at.elapsed() >= timeout {
            login_sessions::set_mistral_login_timeout_if_no_error(
                &state,
                &login_id,
                "timed out waiting for Mistral OAuth completion".to_string(),
            )
            .await;
            common::cleanup_login_home(&paths.login_home).await;
            return;
        }

        match next_mistral_login_event(&state, &login_id, &mut event_rx, &mut progress).await {
            MistralLoginEventOutcome::Pending => {}
            MistralLoginEventOutcome::Failed => {
                common::cleanup_login_home(&paths.login_home).await;
                return;
            }
            MistralLoginEventOutcome::ChannelDisconnected => {
                let message = if progress.observed_auth_url {
                    "Mistral sign-in session ended before completion."
                } else {
                    "Mistral sign-in did not emit an OAuth URL in this environment."
                };
                login_sessions::set_mistral_login_failed_if_no_error(
                    &state,
                    &login_id,
                    message.to_string(),
                )
                .await;
                common::cleanup_login_home(&paths.login_home).await;
                return;
            }
            MistralLoginEventOutcome::Success => {
                complete_mistral_login(&state, &login_id, &label, progress.observed_email.clone())
                    .await;
                common::cleanup_login_home(&paths.login_home).await;
                return;
            }
        }
    }
}

#[derive(Default)]
struct MistralLoginProgress {
    observed_auth_url: bool,
    observed_email: Option<String>,
}

enum MistralLoginEventOutcome {
    Pending,
    Failed,
    ChannelDisconnected,
    Success,
}

async fn next_mistral_login_event(
    state: &Arc<DaemonState>,
    login_id: &str,
    event_rx: &mut mpsc::Receiver<ctx_providers::events::NormalizedEvent>,
    progress: &mut MistralLoginProgress,
) -> MistralLoginEventOutcome {
    let event =
        match tokio::time::timeout(common::MISTRAL_LOGIN_POLL_INTERVAL, event_rx.recv()).await {
            Ok(Some(event)) => event,
            Ok(None) => return MistralLoginEventOutcome::ChannelDisconnected,
            Err(_) => return MistralLoginEventOutcome::Pending,
        };

    if let Some(auth_url) = common::extract_auth_url_from_value(&event.payload_json) {
        progress.observed_auth_url = true;
        login_sessions::set_mistral_login_auth_url(state, login_id, auth_url).await;
    }
    if progress.observed_email.is_none() {
        progress.observed_email = common::first_email_from_value(&event.payload_json);
    }

    if common::is_auth_failure_notice_code(common::auth_notice_code(&event.payload_json)) {
        let message = event
            .payload_json
            .get("message")
            .and_then(serde_json::Value::as_str)
            .map(logs::redact_sensitive)
            .unwrap_or_else(|| "mistral authenticate reported an error".to_string());
        login_sessions::set_mistral_login_failed(state, login_id, message).await;
        return MistralLoginEventOutcome::Failed;
    }

    if matches!(event.event_type, SessionEventType::Notice) {
        let code = common::auth_notice_code(&event.payload_json);
        if common::is_auth_success_notice_code(code) {
            return MistralLoginEventOutcome::Success;
        }
        if common::is_auth_failure_notice_code(code) {
            let message = event
                .payload_json
                .get("message")
                .and_then(serde_json::Value::as_str)
                .map(logs::redact_sensitive)
                .unwrap_or_else(|| "Mistral sign-in failed. Retry.".to_string());
            login_sessions::set_mistral_login_failed(state, login_id, message).await;
            return MistralLoginEventOutcome::Failed;
        }
    }

    MistralLoginEventOutcome::Pending
}

async fn complete_mistral_login(
    state: &Arc<DaemonState>,
    login_id: &str,
    label: &Option<String>,
    observed_email: Option<String>,
) {
    match accounts::upsert_mistral_account_for_login(state, label.clone(), observed_email).await {
        Ok(outcome) => {
            let (_, restart_result) = outcome.into_restart_result();
            login_sessions::finish_mistral_login_session(state, login_id, restart_result).await;
        }
        Err(err) => {
            login_sessions::set_mistral_login_failed(
                state,
                login_id,
                logs::redact_sensitive(&err.auth_login_error_message()),
            )
            .await;
        }
    }
}
