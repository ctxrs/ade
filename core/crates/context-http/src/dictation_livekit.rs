use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use axum::extract::ws::{Message as WsMessage, WebSocket};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use futures::{SinkExt, StreamExt};
use jsonwebtoken::{EncodingKey, Header};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message as TMessage;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;
use url::Url;

use crate::daemon::AppState;
use crate::settings::{self, DictationProvider, LiveKitDictationSettings};

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

#[derive(Debug, Serialize)]
struct LiveKitInferenceClaims {
    iss: String,
    sub: String,
    nbf: usize,
    exp: usize,
    inference: LiveKitInferenceGrant,
}

#[derive(Debug, Serialize)]
struct LiveKitInferenceGrant {
    perform: bool,
}

fn make_inference_token(api_key: &str, api_secret: &str) -> anyhow::Result<String> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as usize;
    let exp = now + Duration::from_secs(600).as_secs() as usize;
    let claims = LiveKitInferenceClaims {
        iss: api_key.to_string(),
        sub: "agent".to_string(),
        nbf: now,
        exp,
        inference: LiveKitInferenceGrant { perform: true },
    };

    Ok(jsonwebtoken::encode(
        &Header::new(jsonwebtoken::Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(api_secret.as_bytes()),
    )?)
}

fn ws_url_for_inference(base_url: &str) -> anyhow::Result<Url> {
    let base_url = base_url.trim().trim_end_matches('/');
    let mut url = Url::parse(base_url).context("invalid base_url")?;
    match url.scheme() {
        "http" => {
            url.set_scheme("ws")
                .map_err(|_| anyhow::anyhow!("failed to set ws scheme"))?;
        }
        "https" => {
            url.set_scheme("wss")
                .map_err(|_| anyhow::anyhow!("failed to set wss scheme"))?;
        }
        "ws" | "wss" => {}
        other => anyhow::bail!("unsupported base_url scheme: {other}"),
    };
    url.set_path(&format!("{}/stt", url.path().trim_end_matches('/')));
    Ok(url)
}

fn normalize_model_id(model: &str) -> String {
    let m = model.trim();
    if m.is_empty() || m.eq_ignore_ascii_case("auto") {
        // LiveKit Inference's STT WebSocket requires an explicit model id (at least on LiveKit Cloud).
        // Default to a strong general-purpose model that supports streaming + interim transcripts.
        return "deepgram/nova-3".to_string();
    }
    if m.eq_ignore_ascii_case("elevenlabs/scribe-v2-realtime") {
        return "elevenlabs/scribe_v2_realtime".to_string();
    }
    if m.eq_ignore_ascii_case("deepgram/flux") {
        return "deepgram/flux-general".to_string();
    }
    m.to_string()
}

async fn connect_livekit_inference_stt(
    cfg: &LiveKitDictationSettings,
) -> anyhow::Result<WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>> {
    let token = make_inference_token(&cfg.api_key, cfg.api_secret.as_deref().unwrap_or(""))?;

    let url = ws_url_for_inference(&cfg.base_url)?;
    let mut req = url.as_str().into_client_request()?;
    req.headers_mut().insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}"))?,
    );

    let (mut ws, _) = tokio_tungstenite::connect_async(req).await?;

    let mut settings = serde_json::Map::new();
    settings.insert(
        "sample_rate".to_string(),
        serde_json::Value::String("16000".to_string()),
    );
    settings.insert(
        "encoding".to_string(),
        serde_json::Value::String("pcm_s16le".to_string()),
    );
    settings.insert(
        "extra".to_string(),
        serde_json::Value::Object(serde_json::Map::new()),
    );
    if !cfg.language.trim().is_empty() {
        settings.insert(
            "language".to_string(),
            serde_json::Value::String(cfg.language.trim().to_string()),
        );
    }

    let mut session_create = serde_json::Map::new();
    session_create.insert(
        "type".to_string(),
        serde_json::Value::String("session.create".to_string()),
    );
    session_create.insert("settings".to_string(), serde_json::Value::Object(settings));
    session_create.insert(
        "model".to_string(),
        serde_json::Value::String(normalize_model_id(&cfg.model)),
    );

    ws.send(TMessage::Text(
        serde_json::Value::Object(session_create).to_string().into(),
    ))
    .await?;
    // tungstenite 0.26 uses an internal Utf8Bytes type
    // (String implements Into<Utf8Bytes>).

    Ok(ws)
}

pub async fn dictation_livekit_stream(mut socket: WebSocket, state: std::sync::Arc<AppState>) {
    tracing::info!("dictation: client connected");
    let settings = settings::load_settings(&state.data_root).await;
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

    let Some(mut cfg) = dictation.livekit else {
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

    if cfg.base_url.trim().is_empty() {
        cfg.base_url = "https://agent-gateway.livekit.cloud/v1".to_string();
    }

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
    let (mut lk_tx, mut lk_rx) = lk_ws.split();

    let finalize_requested = Arc::new(AtomicBool::new(false));
    let finalize_requested_tx = finalize_requested.clone();
    let finalize_requested_rx = finalize_requested.clone();
    let client_tx_send = client_tx.clone();
    let client_tx_recv = client_tx.clone();
    let audio_started = Arc::new(AtomicBool::new(false));
    let audio_started_send = audio_started.clone();

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
                    if lk_tx.send(TMessage::Text(payload.into())).await.is_err() {
                        break;
                    }
                }
                WsMessage::Binary(_) => {}
                WsMessage::Text(text) => {
                    let parsed = serde_json::from_str::<ClientControl>(&text);
                    if matches!(parsed, Ok(ClientControl::Stop)) && !finalized {
                        finalize_requested_tx.store(true, Ordering::Relaxed);
                        let _ = lk_tx
                            .send(TMessage::Text(
                                json!({ "type": "session.finalize" }).to_string().into(),
                            ))
                            .await;
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
                        let _ = lk_tx
                            .send(TMessage::Text(
                                json!({ "type": "session.finalize" }).to_string().into(),
                            ))
                            .await;
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
                        "session.finalized" => break,
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
                        "session.closed" => break,
                        _ => {}
                    }
                }
                TMessage::Close(_) => break,
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
