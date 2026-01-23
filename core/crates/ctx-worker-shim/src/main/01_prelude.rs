use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use std::{env, fmt};

use anyhow::{Context, Result};
use base64::Engine;
use chrono::Utc;
use clap::Parser;
use ctx_fs::patch::{build_worktree_patch, should_ignore_path};
use ctx_worker_protocol::{DiffArtifact, RelayMessage, TerminalControlMessage, WorkerRegistration};
use futures_util::{SinkExt, StreamExt};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, RootCertStore, SignatureScheme};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::Command;
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, connect_async_tls_with_config, Connector};
use tracing::{debug, info, warn};

#[derive(Parser, Debug)]
#[command(name = "ctx-worker-shim")]
struct Args {
    #[arg(long)]
    gateway_url: Option<String>,
    #[arg(long)]
    worker_id: Option<String>,
    #[arg(long)]
    workdir: Option<PathBuf>,
    #[arg(long, default_value = "HEAD")]
    base_commit: String,
    #[arg(long, default_value_t = 1500)]
    diff_debounce_ms: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    if let Err(err) = rustls::crypto::aws_lc_rs::default_provider().install_default() {
        warn!("failed to install rustls crypto provider: {err:?}");
    }

    if std::env::args().nth(1).as_deref() == Some("acp-fake") {
        return run_acp_fake().await;
    }

    let args = Args::parse();
    let args = ResolvedArgs::from_args(args)?;
    let client = gateway_http_client(args.gateway_ca_pem.as_deref())?;

    register_worker(&client, &args).await?;
    emit_diff(&client, &args).await.ok();

    let diff_task = tokio::spawn(run_diff_watcher(client.clone(), args.clone()));
    let acp_task = tokio::spawn(run_acp_relay(args.clone()));
    let terminal_task = tokio::spawn(run_terminal_control(args.clone()));

    let _ = tokio::try_join!(diff_task, acp_task, terminal_task)?;
    Ok(())
}

async fn run_acp_fake() -> Result<()> {
    let stdin = tokio::io::stdin();
    let mut stdout = BufWriter::new(tokio::io::stdout());
    let mut lines = BufReader::new(stdin).lines();

    let mut next_session = 1u64;
    while let Some(line) = lines.next_line().await.context("read acp line")? {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let method = value.get("method").and_then(|v| v.as_str());
        let req_id = value.get("id").cloned();

        match method {
            Some("initialize") => {
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": {
                        "protocolVersion": 1,
                        "agentCapabilities": {
                            "promptCapabilities": {
                                "image": false,
                                "embeddedContext": true,
                            },
                            "loadSession": true,
                        },
                    },
                });
                write_json_line(&mut stdout, &resp).await?;
            }
            Some("session/new") | Some("session/load") => {
                let session_id = value
                    .pointer("/params/sessionId")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| {
                        let id = next_session;
                        next_session += 1;
                        format!("sess_{id}")
                    });
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": {
                        "sessionId": session_id,
                    },
                });
                write_json_line(&mut stdout, &resp).await?;
            }
            Some("session/prompt") => {
                let session_id = value
                    .pointer("/params/sessionId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("sess_1");
                let response_text =
                    extract_expected_reply(&value).unwrap_or_else(|| "ok".to_string());

                let update = serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "session/update",
                    "params": {
                        "sessionId": session_id,
                        "update": {
                            "sessionUpdate": "agent_message_chunk",
                            "content": {
                                "type": "text",
                                "text": response_text,
                            },
                        },
                    },
                });
                write_json_line(&mut stdout, &update).await?;

                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": {
                        "stopReason": "end_turn",
                    },
                });
                write_json_line(&mut stdout, &resp).await?;
            }
            Some("session/cancel") => {}
            _ => {}
        }
    }

    Ok(())
}

fn extract_expected_reply(value: &serde_json::Value) -> Option<String> {
    const PREFIX: &str = "Reply with the exact text:";
    fn walk(value: &serde_json::Value) -> Option<String> {
        match value {
            serde_json::Value::String(s) => {
                let idx = s.find(PREFIX)?;
                let suffix = s[idx + PREFIX.len()..].trim();
                (!suffix.is_empty()).then_some(suffix.to_string())
            }
            serde_json::Value::Array(arr) => arr.iter().find_map(walk),
            serde_json::Value::Object(map) => map.values().find_map(walk),
            _ => None,
        }
    }
    walk(value)
}

async fn write_json_line(
    stdout: &mut BufWriter<tokio::io::Stdout>,
    value: &serde_json::Value,
) -> Result<()> {
    let line = serde_json::to_string(value).context("serialize acp json")?;
    stdout.write_all(line.as_bytes()).await?;
    stdout.write_all(b"\n").await?;
    stdout.flush().await?;
    Ok(())
}

async fn run_diff_watcher(client: reqwest::Client, args: ResolvedArgs) -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Event>();
    let mut watcher = watcher(tx)?;
    watcher
        .watch(&args.workdir, RecursiveMode::Recursive)
        .context("watching workdir")?;

    let debounce = Duration::from_millis(args.diff_debounce_ms);
    let mut pending = false;
    let timer = tokio::time::sleep(debounce);
    tokio::pin!(timer);

    loop {
        tokio::select! {
            Some(event) = rx.recv() => {
                if should_ignore_event(&event) {
                    continue;
                }
                pending = true;
                timer.as_mut().reset(tokio::time::Instant::now() + debounce);
            }
            _ = &mut timer, if pending => {
                pending = false;
                if let Err(err) = emit_diff(&client, &args).await {
                    warn!("failed to emit diff: {err:#}");
                }
            }
        }
    }
}

async fn run_acp_relay(args: ResolvedArgs) -> Result<()> {
    let base = websocket_base(&args.gateway_url);
    let url = format!("{}/workers/{}/acp/worker", base, args.worker_id);
    debug!(url = %url, "connecting acp relay");
    let mut req = url
        .as_str()
        .into_client_request()
        .context("building websocket request")?;
    if let Some(token) = args.gateway_token.as_deref() {
        req.headers_mut().insert(
            "x-ctx-gateway-token",
            token.parse().context("parsing gateway token")?,
        );
    }

    let (ws_stream, _) = if let Some(pem) = args.gateway_ca_pem.as_deref() {
        let connector = gateway_ws_connector(pem)?;
        connect_async_tls_with_config(req, None, false, Some(connector))
            .await
            .context("connecting to gateway acp relay")?
    } else {
        connect_async(req)
            .await
            .context("connecting to gateway acp relay")?
    };
    let (ws_write, mut ws_read) = ws_stream.split();
    let ws_write = std::sync::Arc::new(Mutex::new(ws_write));
    let sessions: std::sync::Arc<Mutex<HashMap<String, SessionRelay>>> =
        std::sync::Arc::new(Mutex::new(HashMap::new()));

    while let Some(msg) = ws_read.next().await {
        let msg = msg.context("reading websocket")?;
        if let Message::Text(text) = msg {
            let relay: RelayMessage = match serde_json::from_str(&text) {
                Ok(relay) => relay,
                Err(err) => {
                    warn!("invalid relay message: {err:#}");
                    continue;
                }
            };
            match relay {
                RelayMessage::Init {
                    session_id,
                    provider_id,
                    model_id,
                    env,
                    workdir,
                } => {
                    let workdir = workdir
                        .as_deref()
                        .map(PathBuf::from)
                        .unwrap_or_else(|| args.workdir.clone());
                    let mut sessions_guard = sessions.lock().await;
                    if sessions_guard.contains_key(&session_id) {
                        continue;
                    }
                    let relay = spawn_acp_session(
                        session_id.clone(),
                        &provider_id,
                        model_id.as_deref(),
                        env,
                        workdir,
                        ws_write.clone(),
                    )
                    .await?;
                    sessions_guard.insert(session_id, relay);
                }
                RelayMessage::Acp {
                    session_id,
                    payload,
                } => {
                    let sessions_guard = sessions.lock().await;
                    if let Some(relay) = sessions_guard.get(&session_id) {
                        let payload = rewrite_cwd(&payload, &relay.workdir);
                        let _ = relay.tx.send(payload);
                    }
                }
                RelayMessage::Close { session_id } => {
                    let mut sessions_guard = sessions.lock().await;
                    if let Some(relay) = sessions_guard.remove(&session_id) {
                        relay.shutdown().await;
                    }
                }
            }
        }
    }

    Ok(())
}

