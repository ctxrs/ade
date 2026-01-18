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

const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TerminalClientMessage {
    Resize { cols: u16, rows: u16 },
    Input { data: String },
}

#[derive(Debug, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TerminalServerMessage {
    Status {
        status: String,
        exit_code: Option<i32>,
    },
}

#[derive(Debug, Clone)]
struct TerminalOpenSpec {
    terminal_id: String,
    shell: String,
    cwd: Option<String>,
    cols: u16,
    rows: u16,
}

struct TerminalHandle {
    shutdown_tx: mpsc::UnboundedSender<()>,
}

async fn run_terminal_control(args: ResolvedArgs) -> Result<()> {
    let base = websocket_base(&args.gateway_url);
    let url = format!(
        "{}/workers/{}/terminals/control/worker",
        base, args.worker_id
    );
    debug!(url = %url, "connecting terminal control");
    let mut req = url
        .as_str()
        .into_client_request()
        .context("building terminal control request")?;
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
            .context("connecting to gateway terminal control")?
    } else {
        connect_async(req)
            .await
            .context("connecting to gateway terminal control")?
    };
    debug!("terminal control connected");
    let (_, mut ws_read) = ws_stream.split();

    let terminals: Arc<Mutex<HashMap<String, TerminalHandle>>> =
        Arc::new(Mutex::new(HashMap::new()));

    while let Some(msg) = ws_read.next().await {
        let msg = msg.context("reading terminal control")?;
        if let Message::Text(text) = msg {
            let control: TerminalControlMessage = match serde_json::from_str(&text) {
                Ok(control) => control,
                Err(err) => {
                    warn!("invalid terminal control message: {err:#}");
                    continue;
                }
            };

            match control {
                TerminalControlMessage::Open {
                    terminal_id,
                    shell,
                    cwd,
                    cols,
                    rows,
                } => {
                    debug!(terminal_id = %terminal_id, "opening terminal");
                    let mut guard = terminals.lock().await;
                    if guard.contains_key(&terminal_id) {
                        continue;
                    }
                    let (shutdown_tx, shutdown_rx) = mpsc::unbounded_channel::<()>();
                    guard.insert(terminal_id.clone(), TerminalHandle { shutdown_tx });
                    let spec = TerminalOpenSpec {
                        terminal_id,
                        shell,
                        cwd,
                        cols,
                        rows,
                    };
                    let args_clone = args.clone();
                    let terminals_clone = terminals.clone();
                    let terminal_id_clone = spec.terminal_id.clone();
                    tokio::spawn(async move {
                        if let Err(err) = run_terminal_session(args_clone, spec, shutdown_rx).await
                        {
                            warn!("terminal session error: {err:#}");
                        }
                        let mut guard = terminals_clone.lock().await;
                        guard.remove(&terminal_id_clone);
                    });
                }
                TerminalControlMessage::Close { terminal_id } => {
                    let mut guard = terminals.lock().await;
                    if let Some(handle) = guard.remove(&terminal_id) {
                        let _ = handle.shutdown_tx.send(());
                    }
                }
            }
        }
    }

    Ok(())
}

async fn run_terminal_session(
    args: ResolvedArgs,
    spec: TerminalOpenSpec,
    mut shutdown_rx: mpsc::UnboundedReceiver<()>,
) -> Result<()> {
    let cols = if spec.cols == 0 {
        DEFAULT_COLS
    } else {
        spec.cols
    };
    let rows = if spec.rows == 0 {
        DEFAULT_ROWS
    } else {
        spec.rows
    };
    let pty_system = NativePtySystem::default();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("open pty")?;

    let mut cwd = args.workdir.clone();
    if let Some(rel) = spec.cwd.as_ref() {
        let candidate = args.workdir.join(rel);
        if let Ok(canon) = std::fs::canonicalize(&candidate) {
            if canon.starts_with(&args.workdir) {
                cwd = canon;
            }
        }
    }

    let mut cmd = CommandBuilder::new(spec.shell.clone());
    cmd.cwd(cwd);
    cmd.env("TERM", "xterm-256color");

    let child = pair.slave.spawn_command(cmd).context("spawn terminal")?;
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().context("clone pty reader")?;
    let mut writer = pair.master.take_writer().context("take pty writer")?;
    let master = Arc::new(StdMutex::new(pair.master));
    let child_arc = Arc::new(StdMutex::new(child));

    let url = format!(
        "{}/workers/{}/terminals/{}/worker",
        websocket_base(&args.gateway_url),
        args.worker_id,
        spec.terminal_id
    );
    debug!(terminal_id = %spec.terminal_id, url = %url, "connecting terminal session");
    let mut req = url
        .as_str()
        .into_client_request()
        .context("building terminal websocket")?;
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
            .context("connecting to gateway terminal relay")?
    } else {
        connect_async(req)
            .await
            .context("connecting to gateway terminal relay")?
    };
    debug!(terminal_id = %spec.terminal_id, "terminal session connected");
    let (mut ws_write, mut ws_read) = ws_stream.split();

    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Message>();
    let send_task = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if ws_write.send(msg).await.is_err() {
                break;
            }
        }
    });

    let out_tx_clone = out_tx.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let _ = out_tx_clone.send(Message::Binary(buf[..n].to_vec().into()));
                }
                Err(_) => break,
            }
        }
    });

    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    std::thread::spawn(move || {
        while let Some(data) = input_rx.blocking_recv() {
            if writer.write_all(&data).is_err() {
                break;
            }
            let _ = writer.flush();
        }
    });

    let out_tx_status = out_tx.clone();
    let child_for_status = child_arc.clone();
    std::thread::spawn(move || loop {
        let exit: Option<portable_pty::ExitStatus> = {
            let mut child = child_for_status.lock().expect("terminal child lock");
            child.try_wait().ok().flatten()
        };
        if let Some(status) = exit {
            let exit_code = i32::try_from(status.exit_code()).ok();
            let payload = serde_json::to_string(&TerminalServerMessage::Status {
                status: "exited".to_string(),
                exit_code,
            })
            .unwrap_or_else(|_| "{\"type\":\"status\",\"status\":\"exited\"}".to_string());
            let _ = out_tx_status.send(Message::Text(payload.into()));
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    });

    loop {
        tokio::select! {
            Some(msg) = ws_read.next() => {
                let msg = msg.context("reading terminal relay")?;
                match msg {
                    Message::Binary(data) => {
                        let _ = input_tx.send(data.to_vec());
                    }
                    Message::Text(text) => {
                        if let Ok(parsed) =
                            serde_json::from_str::<TerminalClientMessage>(text.as_str())
                        {
                            match parsed {
                                TerminalClientMessage::Resize { cols, rows } => {
                                    let master = master.lock().expect("terminal master lock");
                                    let _ = master.resize(PtySize {
                                        rows,
                                        cols,
                                        pixel_width: 0,
                                        pixel_height: 0,
                                    });
                                }
                                TerminalClientMessage::Input { data } => {
                                    let _ = input_tx.send(data.into_bytes());
                                }
                            }
                        } else {
                            let _ = input_tx.send(text.as_bytes().to_vec());
                        }
                    }
                    Message::Close(_) => break,
                    Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
                }
            }
            _ = shutdown_rx.recv() => {
                break;
            }
        }
    }

    {
        let mut child = child_arc.lock().expect("terminal child lock");
        let _ = child.kill();
    }

    drop(input_tx);
    drop(out_tx);
    let _ = send_task.await;
    Ok(())
}

fn watcher(tx: mpsc::UnboundedSender<Event>) -> Result<RecommendedWatcher> {
    let watcher = notify::recommended_watcher(move |res| {
        if let Ok(event) = res {
            let _ = tx.send(event);
        }
    })?;
    Ok(watcher)
}

fn websocket_base(url: &str) -> String {
    let mut base = url.trim_end_matches('/').to_string();
    if base.starts_with("https://") {
        base = base.replacen("https://", "wss://", 1);
    } else if base.starts_with("http://") {
        base = base.replacen("http://", "ws://", 1);
    } else if !base.starts_with("ws://") && !base.starts_with("wss://") {
        base = format!("ws://{base}");
    }
    base
}

#[derive(Debug)]
struct GatewayCertVerifier {
    inner: Arc<WebPkiServerVerifier>,
    server_name: ServerName<'static>,
    pinned_der: Option<Vec<u8>>,
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
        if let Some(pinned) = &self.pinned_der {
            if end_entity.as_ref() == pinned.as_slice() {
                return Ok(ServerCertVerified::assertion());
            }
        }
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
    let mut pinned_der: Option<Vec<u8>> = None;
    for cert in CertificateDer::pem_slice_iter(pem) {
        let cert = cert.context("parsing gateway CA")?;
        if pinned_der.is_none() {
            pinned_der = Some(cert.as_ref().to_vec());
        }
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
        pinned_der,
    };
    let mut config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    config
        .dangerous()
        .set_certificate_verifier(Arc::new(verifier));
    Ok(Connector::Rustls(Arc::new(config)))
}

fn gateway_http_client(gateway_ca_pem: Option<&[u8]>) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder();
    if let Some(pem) = gateway_ca_pem {
        let cert = reqwest::Certificate::from_pem(pem).context("parsing gateway CA certificate")?;
        builder = builder
            .add_root_certificate(cert)
            .danger_accept_invalid_hostnames(true);
    }
    builder.build().context("building gateway http client")
}

fn should_ignore_event(event: &Event) -> bool {
    event.paths.iter().all(|path| should_ignore_path(path))
}

fn acp_log_dir(workdir: &Path) -> PathBuf {
    workdir.join(".ctx").join("worker-logs").join("acp")
}

async fn open_log_writer(path: &Path) -> Option<BufWriter<tokio::fs::File>> {
    if let Some(parent) = path.parent() {
        if let Err(err) = tokio::fs::create_dir_all(parent).await {
            warn!("failed to create log directory {}: {err}", parent.display());
            return None;
        }
    }
    match tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
    {
        Ok(file) => Some(BufWriter::new(file)),
        Err(err) => {
            warn!("failed to open log file {}: {err}", path.display());
            None
        }
    }
}
async fn register_worker(client: &reqwest::Client, args: &ResolvedArgs) -> Result<()> {
    let url = format!("{}/workers/{}/register", args.gateway_url, args.worker_id);
    let acp_log_path = acp_log_dir(&args.workdir);
    let acp_log_dir = if tokio::fs::create_dir_all(&acp_log_path).await.is_ok() {
        Some(acp_log_path.to_string_lossy().to_string())
    } else {
        None
    };
    let reg = WorkerRegistration {
        worker_id: args.worker_id.clone(),
        agent_endpoint: None,
        ssh: None,
        acp_log_dir,
    };

    let mut req = client.post(url).json(&reg);
    if let Some(token) = args.gateway_token.as_deref() {
        req = req.header("x-ctx-gateway-token", token);
    }

    req.send()
        .await
        .context("registering worker")?
        .error_for_status()
        .context("registering worker status")?;

    Ok(())
}

async fn emit_diff(client: &reqwest::Client, args: &ResolvedArgs) -> Result<()> {
    let base = resolve_base_commit(&args.workdir, &args.base_commit).await?;
    let patch = build_worktree_patch(&args.workdir, &base).await?;

    let diff = DiffArtifact {
        worker_id: args.worker_id.clone(),
        base_commit_sha: patch.base_commit_sha,
        head_commit_sha: patch.head_commit_sha,
        generated_at: Utc::now(),
        patch: patch.patch,
        changed_files: patch.changed_files,
        file_count: patch.file_count,
        line_additions: patch.line_additions,
        line_deletions: patch.line_deletions,
    };

    let url = format!("{}/workers/{}/diff", args.gateway_url, args.worker_id);
    let mut req = client.post(url).json(&diff);
    if let Some(token) = args.gateway_token.as_deref() {
        req = req.header("x-ctx-gateway-token", token);
    }

    req.send()
        .await
        .context("sending diff")?
        .error_for_status()
        .context("diff response")?;

    info!("diff emitted");
    Ok(())
}

async fn git_output(workdir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workdir)
        .output()
        .await
        .context("running git")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git command failed: {stderr}");
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

async fn git_rev_parse(workdir: &Path, rev: &str) -> Result<Option<String>> {
    let output = Command::new("git")
        .args(["rev-parse", rev])
        .current_dir(workdir)
        .output()
        .await
        .context("running git")?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    ))
}

async fn resolve_base_commit(workdir: &Path, base: &str) -> Result<String> {
    if git_commit_exists(workdir, base).await? {
        return Ok(base.to_string());
    }
    if let Some(head) = git_rev_parse(workdir, "HEAD").await? {
        warn!(base, head, "base commit missing; falling back to HEAD");
        return Ok(head);
    }
    let empty_tree = git_output(workdir, &["hash-object", "-t", "tree", "/dev/null"]).await?;
    let empty_tree = empty_tree.trim().to_string();
    warn!(base, empty_tree, "base commit missing; using empty tree");
    Ok(empty_tree)
}

async fn git_commit_exists(workdir: &Path, rev: &str) -> Result<bool> {
    let output = Command::new("git")
        .args(["rev-parse", "--verify", &format!("{rev}^{{commit}}")])
        .current_dir(workdir)
        .output()
        .await
        .context("running git")?;
    Ok(output.status.success())
}
#[derive(Clone)]
struct ResolvedArgs {
    gateway_url: String,
    worker_id: String,
    workdir: PathBuf,
    base_commit: String,
    diff_debounce_ms: u64,
    gateway_token: Option<String>,
    gateway_ca_pem: Option<Vec<u8>>,
}

impl ResolvedArgs {
    fn from_args(args: Args) -> Result<Self> {
        let gateway_url = resolve_string(args.gateway_url, "CTX_GATEWAY_URL")?;
        let worker_id = resolve_string(args.worker_id, "CTX_WORKER_ID")?;
        let workdir = resolve_path(args.workdir, "CTX_WORKDIR")?;
        let gateway_token = env::var("CTX_WORKER_GATEWAY_TOKEN")
            .ok()
            .filter(|v| !v.is_empty());
        let gateway_ca_pem = env::var("CTX_GATEWAY_CA_B64")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(|value| {
                base64::engine::general_purpose::STANDARD
                    .decode(value.as_bytes())
                    .context("decoding CTX_GATEWAY_CA_B64")
            })
            .transpose()?;

        Ok(Self {
            gateway_url,
            worker_id,
            workdir,
            base_commit: args.base_commit,
            diff_debounce_ms: args.diff_debounce_ms,
            gateway_token,
            gateway_ca_pem,
        })
    }
}

fn resolve_string(value: Option<String>, env_key: &str) -> Result<String> {
    if let Some(value) = value {
        return Ok(value);
    }
    if let Ok(value) = env::var(env_key) {
        if !value.trim().is_empty() {
            return Ok(value);
        }
    }
    Err(MissingConfig {
        key: env_key.to_string(),
    }
    .into())
}

fn resolve_path(value: Option<PathBuf>, env_key: &str) -> Result<PathBuf> {
    if let Some(value) = value {
        return Ok(value);
    }
    if let Ok(value) = env::var(env_key) {
        let path = PathBuf::from(value);
        return Ok(path);
    }
    Err(MissingConfig {
        key: env_key.to_string(),
    }
    .into())
}

#[derive(Debug)]
struct MissingConfig {
    key: String,
}

impl fmt::Display for MissingConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "missing required config: {}", self.key)
    }
}

impl std::error::Error for MissingConfig {}

struct SessionRelay {
    tx: mpsc::UnboundedSender<String>,
    workdir: PathBuf,
    shutdown_tx: Option<mpsc::UnboundedSender<()>>,
}

impl SessionRelay {
    async fn shutdown(self) {
        if let Some(tx) = self.shutdown_tx {
            let _ = tx.send(());
        }
    }
}

async fn spawn_acp_session(
    session_id: String,
    provider_id: &str,
    model_id: Option<&str>,
    env: HashMap<String, String>,
    workdir: PathBuf,
    ws_write: std::sync::Arc<
        Mutex<
            futures_util::stream::SplitSink<
                tokio_tungstenite::WebSocketStream<
                    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
                >,
                Message,
            >,
        >,
    >,
) -> Result<SessionRelay> {
    let (cmd, args) = provider_command(provider_id);
    let mut command = Command::new(cmd);
    command.args(args);
    if let Some(model_id) = model_id {
        command.env("CTX_MODEL_ID", model_id);
    }
    for (key, value) in env {
        command.env(key, value);
    }
    command.current_dir(&workdir);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn().context("spawning acp provider")?;
    let stdin = child.stdin.take().context("missing stdin")?;
    let stdout = child.stdout.take().context("missing stdout")?;
    let stderr = child.stderr.take().context("missing stderr")?;

    let log_dir = acp_log_dir(&workdir);
    let stdout_path = log_dir.join(format!("{session_id}.stdout.log"));
    let stderr_path = log_dir.join(format!("{session_id}.stderr.log"));
    let stdout_log = open_log_writer(&stdout_path).await;
    let stderr_log = open_log_writer(&stderr_path).await;

    let (line_tx, mut line_rx) = mpsc::unbounded_channel::<String>();
    let (shutdown_tx, mut shutdown_rx) = mpsc::unbounded_channel::<()>();

    let session_id_clone = session_id.clone();
    let ws_write_clone = ws_write.clone();
    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        let mut log_writer = stdout_log;
        while let Ok(Some(line)) = reader.next_line().await {
            if let Some(writer) = log_writer.as_mut() {
                let _ = writer.write_all(line.as_bytes()).await;
                let _ = writer.write_all(b"\n").await;
                let _ = writer.flush().await;
            }
            if tracing::enabled!(tracing::Level::DEBUG) {
                let payload_len = line.len();
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
                    let method = value
                        .get("method")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let update_kind = value
                        .pointer("/params/update/sessionUpdate")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    debug!(
                        session_id = %session_id_clone,
                        method,
                        update_kind,
                        payload_len,
                        "acp stdout relay"
                    );
                } else {
                    debug!(
                        session_id = %session_id_clone,
                        payload_len,
                        "acp stdout relay (unparseable)"
                    );
                }
            }
            let relay = RelayMessage::Acp {
                session_id: session_id_clone.clone(),
                payload: line,
            };
            if let Ok(text) = serde_json::to_string(&relay) {
                let mut sink = ws_write_clone.lock().await;
                if let Err(err) = sink.send(Message::Text(text.into())).await {
                    warn!(
                        session_id = %session_id_clone,
                        error = ?err,
                        "acp relay send failed"
                    );
                    break;
                }
            }
        }
    });

    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        let mut log_writer = stderr_log;
        while let Ok(Some(line)) = reader.next_line().await {
            if let Some(writer) = log_writer.as_mut() {
                let _ = writer.write_all(line.as_bytes()).await;
                let _ = writer.write_all(b"\n").await;
                let _ = writer.flush().await;
            }
            eprintln!("{line}");
        }
    });

    tokio::spawn(async move {
        let mut writer = BufWriter::new(stdin);
        loop {
            tokio::select! {
                Some(line) = line_rx.recv() => {
                    let _ = writer.write_all(line.as_bytes()).await;
                    let _ = writer.write_all(b"\n").await;
                    let _ = writer.flush().await;
                }
                _ = shutdown_rx.recv() => {
                    break;
                }
            }
        }
    });

    Ok(SessionRelay {
        tx: line_tx,
        workdir,
        shutdown_tx: Some(shutdown_tx),
    })
}

fn provider_command(provider_id: &str) -> (&'static str, Vec<&'static str>) {
    match provider_id {
        "fake" => ("/usr/local/bin/ctx-worker-shim", vec!["acp-fake"]),
        "codex" => ("codex-acp", Vec::new()),
        "claude" => ("claude-code-acp", Vec::new()),
        "gemini" => ("gemini", vec!["--experimental-acp"]),
        _ => ("codex-acp", Vec::new()),
    }
}

fn rewrite_cwd(payload: &str, workdir: &Path) -> String {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(payload) else {
        return payload.to_string();
    };
    let method = value.get("method").and_then(|v| v.as_str());
    if matches!(method, Some("session/new")) {
        if let Some(params) = value.get_mut("params") {
            if let Some(map) = params.as_object_mut() {
                map.insert(
                    "cwd".to_string(),
                    serde_json::Value::String(workdir.to_string_lossy().to_string()),
                );
            }
        }
    }
    serde_json::to_string(&value).unwrap_or_else(|_| payload.to_string())
}
