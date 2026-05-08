use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use axum::extract::ws::{Message as WsMessage, WebSocket};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use ctx_transport_runtime::dictation_livekit::{
    connect_livekit_inference_stt, LiveKitDictationConfig,
};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message as TMessage;

use crate::daemon::AppState;
use crate::settings::{self, DictationProvider};

#[derive(Debug, Serialize)]
struct ErrorMsg {
    r#type: &'static str,
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientControl {
    Stop,
}

pub async fn dictation_livekit_stream(mut socket: WebSocket, state: std::sync::Arc<AppState>) {
    tracing::info!("dictation: client connected");
    let settings = match settings::load_settings(state.global_store()).await {
        Ok(settings) => settings,
        Err(err) => {
            let _ = socket
                .send(WsMessage::Text(
                    serde_json::to_string(&ErrorMsg {
                        r#type: "error",
                        message: format!("Failed to load dictation settings: {err}"),
                    })
                    .unwrap_or_else(|_| {
                        "{\"type\":\"error\",\"message\":\"dictation unavailable\"}".to_string()
                    }),
                ))
                .await;
            return;
        }
    };
    let Some(dictation) = settings.dictation else {
        let _ = socket
            .send(WsMessage::Text(
                serde_json::to_string(&ErrorMsg {
                    r#type: "error",
                    message: "Dictation settings not configured.".to_string(),
                })
                .unwrap_or_else(|_| {
                    "{\"type\":\"error\",\"message\":\"dictation unavailable\"}".to_string()
                }),
            ))
            .await;
        return;
    };

    if !dictation.enabled || !matches!(dictation.provider, DictationProvider::LiveKitInference) {
        let _ = socket
            .send(WsMessage::Text(
                serde_json::to_string(&ErrorMsg {
                    r#type: "error",
                    message: "Dictation is disabled.".to_string(),
                })
                .unwrap_or_else(|_| {
                    "{\"type\":\"error\",\"message\":\"dictation disabled\"}".to_string()
                }),
            ))
            .await;
        return;
    }

    let Some(cfg) = dictation.livekit else {
        let _ = socket
            .send(WsMessage::Text(
                serde_json::to_string(&ErrorMsg {
                    r#type: "error",
                    message: "LiveKit dictation settings not configured.".to_string(),
                })
                .unwrap_or_else(|_| {
                    "{\"type\":\"error\",\"message\":\"missing livekit config\"}".to_string()
                }),
            ))
            .await;
        return;
    };

    if cfg.api_key.trim().is_empty()
        || cfg
            .api_secret
            .as_ref()
            .map(|s| s.trim().is_empty())
            .unwrap_or(true)
    {
        let _ = socket
            .send(WsMessage::Text(
                serde_json::to_string(&ErrorMsg {
                    r#type: "error",
                    message: "LiveKit API credentials missing. Configure them in Settings."
                        .to_string(),
                })
                .unwrap_or_else(|_| {
                    "{\"type\":\"error\",\"message\":\"missing credentials\"}".to_string()
                }),
            ))
            .await;
        return;
    }

    let cfg = LiveKitDictationConfig {
        api_key: cfg.api_key,
        api_secret: cfg.api_secret.unwrap_or_default(),
        base_url: cfg.base_url,
        model: cfg.model,
        language: cfg.language,
    };

    let lk_ws = match connect_livekit_inference_stt(&cfg).await {
        Ok(ws) => ws,
        Err(e) => {
            let _ = socket
                .send(WsMessage::Text(
                    serde_json::to_string(&ErrorMsg {
                        r#type: "error",
                        message: format!("Failed to connect to LiveKit Inference STT: {e:#}"),
                    })
                    .unwrap_or_else(|_| {
                        "{\"type\":\"error\",\"message\":\"connect failed\"}".to_string()
                    }),
                ))
                .await;
            return;
        }
    };

    let _ = socket
        .send(WsMessage::Text(json!({ "type": "ready" }).to_string()))
        .await;

    let (client_tx, mut client_rx) = socket.split();
    let client_tx = Arc::new(Mutex::new(client_tx));
    let (lk_tx, mut lk_rx) = lk_ws.split();
    let lk_tx = Arc::new(Mutex::new(lk_tx));

    let finalize_requested = Arc::new(AtomicBool::new(false));
    let finalize_requested_tx = finalize_requested.clone();
    let finalize_requested_rx = finalize_requested.clone();
    let session_closed = Arc::new(AtomicBool::new(false));
    let session_closed_send = session_closed.clone();
    let session_closed_recv = session_closed.clone();
    let client_tx_send = client_tx.clone();
    let client_tx_recv = client_tx.clone();
    let audio_started = Arc::new(AtomicBool::new(false));
    let audio_started_send = audio_started.clone();
    let close_scheduled = Arc::new(AtomicBool::new(false));
    let close_scheduled_tx = close_scheduled.clone();
    let lk_tx_send = lk_tx.clone();
    let lk_tx_close = lk_tx.clone();

    let send_task = tokio::spawn(async move {
        let mut finalized = false;
        let mut audio_bytes_sent: u64 = 0;
        while let Some(Ok(msg)) = client_rx.next().await {
            match msg {
                WsMessage::Binary(bytes) if !finalized => {
                    audio_bytes_sent = audio_bytes_sent.saturating_add(bytes.len() as u64);
                    if !audio_started_send.swap(true, Ordering::Relaxed) {
                        let _ = client_tx_send
                            .lock()
                            .await
                            .send(WsMessage::Text(
                                json!({ "type": "audio_started" }).to_string(),
                            ))
                            .await;
                    }
                    let audio = BASE64.encode(bytes);
                    let payload = json!({ "type": "input_audio", "audio": audio }).to_string();
                    if lk_tx_send
                        .lock()
                        .await
                        .send(TMessage::Text(payload.into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                WsMessage::Binary(_) => {}
                WsMessage::Text(text) => {
                    let parsed = serde_json::from_str::<ClientControl>(&text);
                    if matches!(parsed, Ok(ClientControl::Stop)) && !finalized {
                        finalize_requested_tx.store(true, Ordering::Relaxed);
                        let _ = lk_tx_send
                            .lock()
                            .await
                            .send(TMessage::Text(
                                json!({ "type": "session.finalize" }).to_string().into(),
                            ))
                            .await;
                        if !close_scheduled_tx.swap(true, Ordering::Relaxed) {
                            let lk_tx_close = lk_tx_close.clone();
                            let session_closed_tx = session_closed_send.clone();
                            tokio::spawn(async move {
                                tokio::time::sleep(Duration::from_millis(150)).await;
                                if session_closed_tx.load(Ordering::Relaxed) {
                                    return;
                                }
                                let _ = lk_tx_close.lock().await.send(TMessage::Close(None)).await;
                            });
                        }
                        finalized = true;
                    }
                }
                WsMessage::Ping(payload) => {
                    let _ = client_tx_send
                        .lock()
                        .await
                        .send(WsMessage::Pong(payload))
                        .await;
                }
                WsMessage::Close(_) => {
                    if !finalized {
                        finalize_requested_tx.store(true, Ordering::Relaxed);
                        let _ = lk_tx_send
                            .lock()
                            .await
                            .send(TMessage::Text(
                                json!({ "type": "session.finalize" }).to_string().into(),
                            ))
                            .await;
                        if !close_scheduled_tx.swap(true, Ordering::Relaxed) {
                            let lk_tx_close = lk_tx_close.clone();
                            let session_closed_tx = session_closed_send.clone();
                            tokio::spawn(async move {
                                tokio::time::sleep(Duration::from_millis(150)).await;
                                if session_closed_tx.load(Ordering::Relaxed) {
                                    return;
                                }
                                let _ = lk_tx_close.lock().await.send(TMessage::Close(None)).await;
                            });
                        }
                    }
                    break;
                }
                _ => {}
            }
        }
        tracing::info!(
            "dictation: client send loop ended finalized={} bytes={}",
            finalized,
            audio_bytes_sent
        );
    });

    let recv_task = tokio::spawn(async move {
        let mut got_final = false;
        let mut last_transcript_at = Instant::now();
        let mut transcript_messages: u64 = 0;

        while let Some(item) = {
            if finalize_requested_rx.load(Ordering::Relaxed) {
                let idle_timeout = if got_final {
                    Duration::from_secs(5)
                } else {
                    Duration::from_secs(60)
                };
                tokio::time::timeout(idle_timeout, lk_rx.next())
                    .await
                    .unwrap_or_default()
            } else {
                lk_rx.next().await
            }
        } {
            let Ok(msg) = item else { break };
            match msg {
                TMessage::Text(text) => {
                    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
                        continue;
                    };
                    let msg_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    match msg_type {
                        "interim_transcript" | "final_transcript" => {
                            transcript_messages = transcript_messages.saturating_add(1);
                            last_transcript_at = Instant::now();
                            if msg_type == "final_transcript" {
                                got_final = true;
                            }
                            let out = json!({
                                "type": if msg_type == "final_transcript" { "final" } else { "interim" },
                                "text": v.get("transcript").and_then(|t| t.as_str()).unwrap_or(""),
                                "language": v.get("language").and_then(|t| t.as_str()).unwrap_or(""),
                            });
                            if client_tx_recv
                                .lock()
                                .await
                                .send(WsMessage::Text(out.to_string()))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        "session.finalized" => {
                            session_closed_recv.store(true, Ordering::Relaxed);
                            break;
                        }
                        "error" => {
                            let out = json!({
                                "type": "error",
                                "message": v.get("message").and_then(|t| t.as_str()).unwrap_or("LiveKit STT error"),
                            });
                            let _ = client_tx_recv
                                .lock()
                                .await
                                .send(WsMessage::Text(out.to_string()))
                                .await;
                            break;
                        }
                        "session.closed" => {
                            session_closed_recv.store(true, Ordering::Relaxed);
                            break;
                        }
                        _ => {}
                    }
                }
                TMessage::Close(_) => {
                    session_closed_recv.store(true, Ordering::Relaxed);
                    break;
                }
                _ => {}
            }

            if finalize_requested_rx.load(Ordering::Relaxed)
                && got_final
                && last_transcript_at.elapsed() > Duration::from_secs(5)
            {
                break;
            }
        }
        tracing::info!(
            "dictation: livekit recv loop ended got_final={} transcripts={} audio_started={}",
            got_final,
            transcript_messages,
            audio_started.load(Ordering::Relaxed)
        );
        let _ = client_tx_recv
            .lock()
            .await
            .send(WsMessage::Text(json!({ "type": "done" }).to_string()))
            .await;
    });

    let _ = tokio::join!(send_task, recv_task);
    tracing::info!("dictation: client disconnected");
}
