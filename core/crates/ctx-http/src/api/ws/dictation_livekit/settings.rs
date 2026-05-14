use axum::extract::ws::{Message as WsMessage, WebSocket};
use ctx_settings_model::DictationProvider;
use ctx_transport_runtime::dictation_livekit::{
    normalize_livekit_dictation_config, LiveKitDictationConfig, LiveKitDictationConfigInput,
};
use serde::Serialize;

use ctx_daemon::daemon::CoreHandle;

#[derive(Debug)]
pub(super) struct DictationStreamError {
    message: String,
    fallback_json: &'static str,
}

impl DictationStreamError {
    pub(super) fn new(message: impl Into<String>, fallback_json: &'static str) -> Self {
        Self {
            message: message.into(),
            fallback_json,
        }
    }
}

#[derive(Debug, Serialize)]
struct ErrorMsg {
    r#type: &'static str,
    message: String,
}

pub(super) async fn send_dictation_error(socket: &mut WebSocket, error: DictationStreamError) {
    let _ = socket
        .send(WsMessage::Text(
            serde_json::to_string(&ErrorMsg {
                r#type: "error",
                message: error.message,
            })
            .unwrap_or_else(|_| error.fallback_json.to_string()),
        ))
        .await;
}

pub(super) async fn load_livekit_dictation_config(
    state: &CoreHandle,
) -> Result<LiveKitDictationConfig, DictationStreamError> {
    let settings = state.load_settings().await.map_err(|err| {
        DictationStreamError::new(
            format!("Failed to load dictation settings: {err}"),
            "{\"type\":\"error\",\"message\":\"dictation unavailable\"}",
        )
    })?;
    let Some(dictation) = settings.dictation else {
        return Err(DictationStreamError::new(
            "Dictation settings not configured.",
            "{\"type\":\"error\",\"message\":\"dictation unavailable\"}",
        ));
    };

    if !dictation.enabled || !matches!(dictation.provider, DictationProvider::LiveKitInference) {
        return Err(DictationStreamError::new(
            "Dictation is disabled.",
            "{\"type\":\"error\",\"message\":\"dictation disabled\"}",
        ));
    }

    let Some(cfg) = dictation.livekit else {
        return Err(DictationStreamError::new(
            "LiveKit dictation settings not configured.",
            "{\"type\":\"error\",\"message\":\"missing livekit config\"}",
        ));
    };

    normalize_livekit_dictation_config(LiveKitDictationConfigInput {
        api_key: cfg.api_key,
        api_secret: cfg.api_secret,
        base_url: cfg.base_url,
        model: cfg.model,
        language: cfg.language,
    })
    .map_err(|err| {
        DictationStreamError::new(
            err.to_string(),
            "{\"type\":\"error\",\"message\":\"missing credentials\"}",
        )
    })
}
