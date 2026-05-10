use super::*;
use ctx_providers::events::NormalizedEvent;

#[derive(Default)]
pub(super) struct AmpLoginProgress {
    pub(super) observed_auth_url: bool,
    pub(super) observed_email: Option<String>,
}

pub(super) enum AmpLoginEventOutcome {
    Pending,
    Failed,
    ChannelDisconnected,
    Success,
}

pub(super) async fn next_amp_login_event(
    state: &Arc<AppState>,
    login_id: &str,
    event_rx: &mut mpsc::Receiver<NormalizedEvent>,
    progress: &mut AmpLoginProgress,
) -> AmpLoginEventOutcome {
    let event = match tokio::time::timeout(AMP_LOGIN_POLL_INTERVAL, event_rx.recv()).await {
        Ok(Some(event)) => event,
        Ok(None) => return AmpLoginEventOutcome::ChannelDisconnected,
        Err(_) => return AmpLoginEventOutcome::Pending,
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
            .unwrap_or_else(|| "amp authenticate reported an error".to_string());
        status::set_failed(state, login_id, message).await;
        return AmpLoginEventOutcome::Failed;
    }

    if matches!(event.event_type, ctx_core::models::SessionEventType::Notice) {
        let code = auth_notice_code(&event.payload_json);
        if is_auth_success_notice_code(code) {
            return AmpLoginEventOutcome::Success;
        }
        if matches!(code, "auth_failed" | "auth_error" | "auth_required") {
            let message = event
                .payload_json
                .get("message")
                .and_then(serde_json::Value::as_str)
                .map(logs::redact_sensitive)
                .unwrap_or_else(|| "Amp sign-in failed. Retry.".to_string());
            status::set_failed(state, login_id, message).await;
            return AmpLoginEventOutcome::Failed;
        }
    }

    AmpLoginEventOutcome::Pending
}
