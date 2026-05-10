use std::sync::Arc;

use tokio::sync::mpsc;

use super::status;
use crate::api::providers::login::{
    auth_notice_code, extract_auth_url_from_value, is_auth_failure_notice_code,
};
use crate::daemon::AppState;
use ctx_observability::logs;
use ctx_providers::events::NormalizedEvent;

pub(super) struct GeminiLoginEventOutcome {
    pub(super) observed_auth_url: bool,
    pub(super) channel_disconnected: bool,
    pub(super) failed: bool,
}

pub(super) async fn drain_gemini_login_events(
    state: &Arc<AppState>,
    login_id: &str,
    event_rx: &mut mpsc::Receiver<NormalizedEvent>,
    mut observed_auth_url: bool,
) -> GeminiLoginEventOutcome {
    let mut channel_disconnected = false;
    loop {
        match event_rx.try_recv() {
            Ok(event) => {
                if let Some(auth_url) = extract_auth_url_from_value(&event.payload_json) {
                    observed_auth_url = true;
                    status::set_auth_url(state, login_id, auth_url).await;
                }
                if is_auth_failure_notice_code(auth_notice_code(&event.payload_json)) {
                    let message = event
                        .payload_json
                        .get("message")
                        .and_then(serde_json::Value::as_str)
                        .map(logs::redact_sensitive)
                        .unwrap_or_else(|| "gemini authenticate reported an error".to_string());
                    status::set_failed(state, login_id, message).await;
                    return GeminiLoginEventOutcome {
                        observed_auth_url,
                        channel_disconnected,
                        failed: true,
                    };
                }
            }
            Err(mpsc::error::TryRecvError::Empty) => break,
            Err(mpsc::error::TryRecvError::Disconnected) => {
                channel_disconnected = true;
                break;
            }
        }
    }
    GeminiLoginEventOutcome {
        observed_auth_url,
        channel_disconnected,
        failed: false,
    }
}
