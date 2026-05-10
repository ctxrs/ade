use super::*;
use ctx_providers::events::NormalizedEvent;

#[derive(Default)]
pub(super) struct MistralLoginProgress {
    pub(super) observed_auth_url: bool,
    pub(super) observed_email: Option<String>,
}

pub(super) enum MistralLoginEventOutcome {
    Pending,
    Failed,
    ChannelDisconnected,
    Success,
}

pub(super) async fn next_mistral_login_event(
    state: &Arc<AppState>,
    login_id: &str,
    event_rx: &mut mpsc::Receiver<NormalizedEvent>,
    progress: &mut MistralLoginProgress,
) -> MistralLoginEventOutcome {
    let event = match tokio::time::timeout(MISTRAL_LOGIN_POLL_INTERVAL, event_rx.recv()).await {
        Ok(Some(event)) => event,
        Ok(None) => return MistralLoginEventOutcome::ChannelDisconnected,
        Err(_) => return MistralLoginEventOutcome::Pending,
    };

    if let Some(auth_url) = extract_auth_url_from_value(&event.payload_json) {
        progress.observed_auth_url = true;
        status::set_auth_url(state, login_id, auth_url).await;
    }
    if progress.observed_email.is_none() {
        progress.observed_email = first_email_from_value(&event.payload_json);
    }

    if is_auth_failure_notice_code(auth_notice_code(&event.payload_json)) {
        let message = event
            .payload_json
            .get("message")
            .and_then(serde_json::Value::as_str)
            .map(logs::redact_sensitive)
            .unwrap_or_else(|| "mistral authenticate reported an error".to_string());
        status::set_failed(state, login_id, message).await;
        return MistralLoginEventOutcome::Failed;
    }

    if matches!(event.event_type, ctx_core::models::SessionEventType::Notice) {
        let code = auth_notice_code(&event.payload_json);
        if is_auth_success_notice_code(code) {
            return MistralLoginEventOutcome::Success;
        }
        if is_auth_failure_notice_code(code) {
            let message = event
                .payload_json
                .get("message")
                .and_then(serde_json::Value::as_str)
                .map(logs::redact_sensitive)
                .unwrap_or_else(|| "Mistral sign-in failed. Retry.".to_string());
            status::set_failed(state, login_id, message).await;
            return MistralLoginEventOutcome::Failed;
        }
    }

    MistralLoginEventOutcome::Pending
}
