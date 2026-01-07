use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, RootCertStore, SignatureScheme};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, connect_async_tls_with_config, Connector};

use ctx_core::models::SessionEventType;
use ctx_worker_protocol::RelayMessage;

use crate::acp::{normalize_session_update, StreamState};
use crate::adapters::{RunHandle, TurnInput};
use crate::events::NormalizedEvent;
use crate::tier1::build_acp_client_config;

const GATEWAY_ENV_URL: &str = "CTX_WORKER_GATEWAY_URL";
const GATEWAY_ENV_WORKER: &str = "CTX_WORKER_ID";
const GATEWAY_ENV_TOKEN: &str = "CTX_WORKER_GATEWAY_TOKEN";
const GATEWAY_ENV_CA: &str = "CTX_WORKER_GATEWAY_CA_B64";

type WsSink = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;

#[derive(Debug)]
struct GatewayCertVerifier {
    inner: Arc<WebPkiServerVerifier>,
    server_name: ServerName<'static>,
}

impl ServerCertVerifier for GatewayCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        self.inner.verify_server_cert(
            end_entity,
            intermediates,
            &self.server_name,
            ocsp_response,
            now,
        )
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

fn gateway_ws_connector(pem: &[u8]) -> Result<Connector> {
    let mut roots = RootCertStore::empty();
    for cert in CertificateDer::pem_slice_iter(pem) {
        let cert = cert.context("parsing gateway CA")?;
        roots.add(cert).context("adding gateway CA")?;
    }
    let verifier = WebPkiServerVerifier::builder(Arc::new(roots.clone()))
        .build()
        .context("building gateway verifier")?;
    let server_name =
        ServerName::try_from("ctx-gateway").context("building gateway server name")?;
    let verifier = GatewayCertVerifier {
        inner: verifier,
        server_name,
    };
    let mut config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    config
        .dangerous()
        .set_certificate_verifier(Arc::new(verifier));
    Ok(Connector::Rustls(Arc::new(config)))
}

pub async fn run_remote_prompt(
    provider_id: String,
    input: TurnInput,
    workdir: PathBuf,
    env: HashMap<String, String>,
    event_sink: mpsc::Sender<NormalizedEvent>,
) -> Result<RunHandle> {
    let gateway_url = env
        .get(GATEWAY_ENV_URL)
        .cloned()
        .context("missing CTX_WORKER_GATEWAY_URL")?;
    let worker_id = env
        .get(GATEWAY_ENV_WORKER)
        .cloned()
        .context("missing CTX_WORKER_ID")?;
    let gateway_ca_pem = env
        .get(GATEWAY_ENV_CA)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| {
            base64::engine::general_purpose::STANDARD
                .decode(value.as_bytes())
                .context("decoding CTX_WORKER_GATEWAY_CA_B64")
        })
        .transpose()?;

    let session_id = env
        .get("CTX_SESSION_ID")
        .cloned()
        .unwrap_or_else(|| "unknown-session".to_string());
    let model_id = env.get("CTX_MODEL_ID").cloned();

    let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
    let (done_tx, done_rx) = oneshot::channel::<()>();

    let join: tokio::task::JoinHandle<Result<()>> = tokio::spawn(async move {
        let client = build_acp_client_config(&env);
        let token = env.get(GATEWAY_ENV_TOKEN).map(|v| v.as_str());
        let (ws_write, mut inbound) = connect_gateway(
            &gateway_url,
            &worker_id,
            &session_id,
            token,
            gateway_ca_pem.as_deref(),
        )
        .await?;

        let init_msg = RelayMessage::Init {
            session_id: session_id.clone(),
            provider_id: provider_id.clone(),
            model_id: model_id.clone(),
            env: env.clone(),
            workdir: None,
        };
        send_relay(&ws_write, init_msg).await?;

        let mut stream_state = StreamState::default();
        let (acp_session_id, prompt_resp, cancel_task) = {
            let mut request_ctx = RequestContext {
                ws_write: &ws_write,
                session_id: &session_id,
                inbound: &mut inbound,
                state: &mut stream_state,
                event_sink: &event_sink,
                provider_id: &provider_id,
            };
            let init_resp = request_ctx
                .send_request(
                    1,
                    "initialize",
                    json!({
                        "protocolVersion": 1,
                        "clientCapabilities": client.client_capabilities,
                        "clientInfo": {
                            "name": client.client_name,
                            "title": client.client_title,
                            "version": client.client_version,
                        }
                    }),
                )
                .await?;

            let auth_methods = init_resp
                .get("result")
                .and_then(|v| v.get("authMethods").or_else(|| v.get("auth_methods")))
                .cloned();
            let capabilities = init_resp
                .get("result")
                .and_then(|v| v.get("capabilities").or_else(|| v.get("agentCapabilities")));
            let supports_load = capabilities
                .and_then(|v| v.get("loadSession"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let supports_resume = capabilities
                .and_then(|v| {
                    v.get("sessionCapabilities")
                        .or_else(|| v.get("session_capabilities"))
                })
                .and_then(|v| v.get("resume"))
                .map(|v| match v {
                    Value::Bool(value) => *value,
                    Value::Null => false,
                    _ => true,
                })
                .unwrap_or(false);

            let mcp_servers = client
                .mcp_servers
                .iter()
                .map(|srv| {
                    json!({
                        "name": srv.name,
                        "command": srv.command,
                        "args": srv.args,
                        "env": srv.env,
                    })
                })
                .collect::<Vec<_>>();
            let session_resp = request_ctx
                .send_request(
                    2,
                    "session/new",
                    json!({"cwd": workdir.to_string_lossy().to_string(), "mcpServers": mcp_servers}),
                )
                .await?;

            if let Some(err) = session_resp.get("error") {
                let _ = event_sink
                    .send(NormalizedEvent {
                        event_type: SessionEventType::Error,
                        payload_json: json!({"provider": provider_id, "acp_error": err}),
                    })
                    .await;
                let _ = event_sink
                    .send(NormalizedEvent {
                        event_type: SessionEventType::Done,
                        payload_json: json!({"provider": provider_id, "status": "error"}),
                    })
                    .await;
                return Ok(());
            }

            let acp_session_id = session_resp
                .get("result")
                .and_then(|v| v.get("sessionId"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let modes = session_resp
                .get("result")
                .and_then(|v| v.get("modes"))
                .cloned();
            let models = session_resp
                .get("result")
                .and_then(|v| v.get("models"))
                .cloned();

            let _ = event_sink
                .send(NormalizedEvent {
                    event_type: SessionEventType::Init,
                    payload_json: json!({
                        "provider": provider_id,
                        "acp_session_id": acp_session_id,
                        "resumed": false,
                        "supports_load": supports_load,
                        "supports_resume": supports_resume,
                        "modes": modes,
                        "models": models,
                        "auth_methods": auth_methods.clone(),
                        "authMethods": auth_methods,
                    }),
                })
                .await;

            let prompt = build_prompt(&input, &env, &provider_id, &event_sink).await?;

            let acp_session_id_for_cancel = acp_session_id.clone();
            let cancel_task = tokio::spawn({
                let ws_write = ws_write.clone();
                let session_id = session_id.clone();
                async move {
                    if cancel_rx.await.is_ok() && !acp_session_id_for_cancel.is_empty() {
                        let _ = send_notification(
                            &ws_write,
                            &session_id,
                            json!({
                                "jsonrpc": "2.0",
                                "method": "session/cancel",
                                "params": { "sessionId": acp_session_id_for_cancel }
                            }),
                        )
                        .await;
                    }
                }
            });

            let prompt_resp = request_ctx
                .send_request(
                    3,
                    "session/prompt",
                    json!({"sessionId": acp_session_id.clone(), "prompt": prompt}),
                )
                .await?;

            (acp_session_id, prompt_resp, cancel_task)
        };

        let _ = cancel_task.await;

        if let Some(done_events) = drain_pending_updates(&mut inbound, &mut stream_state) {
            for ev in done_events {
                let _ = event_sink.send(ev).await;
            }
        }

        if !stream_state.saw_assistant_complete && !stream_state.assistant_buf.trim().is_empty() {
            let _ = event_sink
                .send(NormalizedEvent {
                    event_type: SessionEventType::AssistantComplete,
                    payload_json: json!({"full_content": stream_state.assistant_buf}),
                })
                .await;
        }

        let stop_reason = prompt_resp
            .get("result")
            .and_then(|v| v.get("stopReason"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let _ = event_sink
            .send(NormalizedEvent {
                event_type: SessionEventType::Done,
                payload_json: json!({
                    "provider": provider_id,
                    "acp_session_id": acp_session_id,
                    "status": if prompt_resp.get("error").is_some() { "error" } else { "success" },
                    "stop_reason": stop_reason,
                }),
            })
            .await;

        let _ = send_relay(&ws_write, RelayMessage::Close { session_id }).await;

        Ok(())
    });

    let abort = join.abort_handle();
    drop(tokio::spawn(async move {
        let _ = join.await;
        let _ = done_tx.send(());
    }));

    Ok(RunHandle {
        done: done_rx,
        cancel: Some(cancel_tx),
        abort: Some(abort),
    })
}

async fn connect_gateway(
    gateway_url: &str,
    worker_id: &str,
    session_id: &str,
    token: Option<&str>,
    gateway_ca_pem: Option<&[u8]>,
) -> Result<(Arc<Mutex<WsSink>>, mpsc::UnboundedReceiver<String>)> {
    let mut base = gateway_url.trim_end_matches('/').to_string();
    if base.starts_with("https://") {
        base = base.replacen("https://", "wss://", 1);
    } else if base.starts_with("http://") {
        base = base.replacen("http://", "ws://", 1);
    } else if !base.starts_with("ws://") && !base.starts_with("wss://") {
        base = format!("ws://{base}");
    }
    let url = format!("{base}/workers/{worker_id}/acp/daemon");

    let mut req = url.as_str().into_client_request()?;
    if let Some(token) = token {
        req.headers_mut().insert(
            "x-ctx-gateway-token",
            token.parse().context("parsing gateway token")?,
        );
    }

    let (ws_stream, _) = if let Some(pem) = gateway_ca_pem {
        let connector = gateway_ws_connector(pem)?;
        connect_async_tls_with_config(req, None, false, Some(connector))
            .await
            .context("connecting to gateway")?
    } else {
        connect_async(req).await.context("connecting to gateway")?
    };
    let (ws_write, mut ws_read) = ws_stream.split();
    let (tx, rx) = mpsc::unbounded_channel::<String>();
    let session_id = session_id.to_string();

    tokio::spawn(async move {
        while let Some(msg) = ws_read.next().await {
            if let Ok(Message::Text(text)) = msg {
                if let Ok(RelayMessage::Acp {
                    session_id: sid,
                    payload,
                }) = serde_json::from_str::<RelayMessage>(&text)
                {
                    if sid == session_id {
                        let _ = tx.send(payload);
                    }
                }
            }
        }
    });

    Ok((Arc::new(Mutex::new(ws_write)), rx))
}

async fn send_relay(ws_write: &Arc<Mutex<WsSink>>, message: RelayMessage) -> Result<()> {
    let text = serde_json::to_string(&message)?;
    let mut writer = ws_write.lock().await;
    writer.send(Message::Text(text.into())).await?;
    Ok(())
}

async fn send_notification(
    ws_write: &Arc<Mutex<WsSink>>,
    session_id: &str,
    payload: Value,
) -> Result<()> {
    let msg = RelayMessage::Acp {
        session_id: session_id.to_string(),
        payload: serde_json::to_string(&payload)?,
    };
    send_relay(ws_write, msg).await
}

struct RequestContext<'a> {
    ws_write: &'a Arc<Mutex<WsSink>>,
    session_id: &'a str,
    inbound: &'a mut mpsc::UnboundedReceiver<String>,
    state: &'a mut StreamState,
    event_sink: &'a mpsc::Sender<NormalizedEvent>,
    provider_id: &'a str,
}

impl<'a> RequestContext<'a> {
    async fn send_request(&mut self, id: u64, method: &str, params: Value) -> Result<Value> {
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        send_notification(self.ws_write, self.session_id, req).await?;

        while let Some(payload) = self.inbound.recv().await {
            let Ok(msg) = serde_json::from_str::<Value>(&payload) else {
                continue;
            };
            if msg.get("method") == Some(&Value::String("session/update".to_string())) {
                let events = normalize_session_update(&msg, self.state);
                for ev in events {
                    let _ = self.event_sink.send(ev).await;
                }
                continue;
            }
            if jsonrpc_id(&msg) == Some(id) {
                return Ok(msg);
            }
        }

        let _ = self
            .event_sink
            .send(NormalizedEvent {
                event_type: SessionEventType::Error,
                payload_json: json!({
                    "provider": self.provider_id,
                    "message": "gateway connection closed"
                }),
            })
            .await;

        anyhow::bail!("gateway connection closed")
    }
}

fn jsonrpc_id(msg: &Value) -> Option<u64> {
    msg.get("id").and_then(|v| {
        v.as_u64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
    })
}

async fn build_prompt(
    input: &TurnInput,
    env: &HashMap<String, String>,
    provider_id: &str,
    event_sink: &mpsc::Sender<NormalizedEvent>,
) -> Result<Vec<Value>> {
    let mut prompt: Vec<Value> = Vec::new();

    if !input.context_blocks.is_empty() {
        prompt.extend(input.context_blocks.clone());
    }

    let data_root = env.get("CTX_DATA_ROOT").cloned();
    for att in input.attachments.iter() {
        match att {
            ctx_core::models::MessageAttachment::Image {
                mime_type,
                data_base64,
                ..
            } => {
                prompt.push(json!({"type":"image","data": data_base64, "mimeType": mime_type}));
            }
            ctx_core::models::MessageAttachment::ImageRef {
                blob_id, mime_type, ..
            } => {
                let Some(data_root) = data_root.as_deref() else {
                    let _ = event_sink
                        .send(NormalizedEvent {
                            event_type: SessionEventType::Error,
                            payload_json: json!({"provider": provider_id, "message": "missing CTX_DATA_ROOT for image attachment"}),
                        })
                        .await;
                    continue;
                };
                let path = std::path::Path::new(data_root).join("blobs").join(blob_id);
                let bytes = match tokio::fs::read(&path).await {
                    Ok(b) => b,
                    Err(e) => {
                        let _ = event_sink
                            .send(NormalizedEvent {
                                event_type: SessionEventType::Error,
                                payload_json: json!({"provider": provider_id, "message": format!("failed to read image blob {blob_id}: {e}")}),
                            })
                            .await;
                        continue;
                    }
                };
                let data_base64 = base64::engine::general_purpose::STANDARD.encode(bytes);
                prompt.push(json!({"type":"image","data": data_base64, "mimeType": mime_type}));
            }
        }
    }

    prompt.push(json!({"type":"text","text": input.content}));
    Ok(prompt)
}

fn drain_pending_updates(
    inbound: &mut mpsc::UnboundedReceiver<String>,
    state: &mut StreamState,
) -> Option<Vec<NormalizedEvent>> {
    let mut out = Vec::new();
    while let Ok(payload) = inbound.try_recv() {
        let Ok(msg) = serde_json::from_str::<Value>(&payload) else {
            continue;
        };
        if msg.get("method") == Some(&Value::String("session/update".to_string())) {
            out.extend(normalize_session_update(&msg, state));
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}
