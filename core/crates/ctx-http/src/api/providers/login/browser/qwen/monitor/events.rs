use super::*;
use ctx_providers::events::NormalizedEvent;

#[derive(Default)]
pub(super) struct QwenLoginProgress {
    pub(super) observed_auth_url: bool,
    pub(super) observed_email: Option<String>,
}

pub(super) struct QwenLoginEventOutcome {
    pub(super) channel_disconnected: bool,
    pub(super) failed: bool,
}

pub(super) async fn drain_qwen_login_events(
    providers: &ProvidersHandle,
    login_id: &str,
    event_rx: &mut mpsc::Receiver<NormalizedEvent>,
    progress: &mut QwenLoginProgress,
) -> QwenLoginEventOutcome {
    let mut channel_disconnected = false;
    loop {
        match event_rx.try_recv() {
            Ok(event) => {
                if let Some(auth_url) = extract_auth_url_from_value(&event.payload_json) {
                    progress.observed_auth_url = true;
                    status::set_auth_url(providers, login_id, auth_url).await;
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
                        .unwrap_or_else(|| "qwen authenticate reported an error".to_string());
                    status::set_failed(providers, login_id, message).await;
                    return QwenLoginEventOutcome {
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

    QwenLoginEventOutcome {
        channel_disconnected,
        failed: false,
    }
}
