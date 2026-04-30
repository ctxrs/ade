use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{FromRequestParts, Path, State};
use axum::http::{HeaderMap, Method, Request, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use base64::Engine;
use chrono::Utc;
use clap::Parser;
use ctx_tunnel_store::{RelayAssignmentValidation, RelayHeartbeat, RelayRegistration, TunnelStore};
use futures::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use tracing::{info, warn};
use uuid::Uuid;

mod http_proxy;

use crate::http_proxy::extract_forward_headers;

#[derive(Parser, Debug)]
#[command(name = "ctx-tunnel-relay", version)]
struct Args {
    #[arg(long, default_value = "127.0.0.1:8787")]
    listen: String,
}

#[derive(Clone)]
struct RelayState {
    tunnels: Arc<RwLock<HashMap<String, Arc<Tunnel>>>>,
    master_secret: Vec<u8>,
    store: TunnelStore,
    relay_id: String,
}

struct Tunnel {
    inner: Mutex<TunnelInner>,
}

struct TunnelInner {
    desktop: Option<DesktopHandle>,
    pending_http: HashMap<String, oneshot::Sender<HttpResponse>>,
    pending_ws_open: HashMap<String, oneshot::Sender<Result<(), String>>>,
    ws_streams: HashMap<String, WsStreamHandle>,
}

#[derive(Clone)]
struct DesktopHandle {
    tx: mpsc::UnboundedSender<RelayToClient>,
}

#[derive(Clone)]
struct WsStreamHandle {
    mobile_tx: mpsc::UnboundedSender<Message>,
}

const TUNNEL_SECRET_HEADER: &str = "x-ctx-tunnel-secret";
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum RelayToClient {
    HttpRequest {
        id: String,
        method: String,
        path: String,
        headers: Vec<(String, String)>,
        body_b64: String,
    },
    WsOpen {
        id: String,
        path: String,
        headers: Vec<(String, String)>,
    },
    WsMessage {
        id: String,
        is_binary: bool,
        data: String,
    },
    WsClose {
        id: String,
        code: Option<u16>,
        reason: Option<String>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientToRelay {
    HttpResponse {
        id: String,
        status: u16,
        headers: Vec<(String, String)>,
        body_b64: String,
    },
    WsOpenResult {
        id: String,
        ok: bool,
        #[serde(default)]
        error: Option<String>,
    },
    WsMessage {
        id: String,
        is_binary: bool,
        data: String,
    },
    WsClosed {
        id: String,
        #[serde(default)]
        code: Option<u16>,
        #[serde(default)]
        reason: Option<String>,
    },
}

#[derive(Debug)]
struct HttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let master_secret = load_master_secret()?;
    let config = load_relay_config()?;
    let store = TunnelStore::connect(&config.database_url)
        .await
        .context("connecting to mobile tunnel database")?;
    store
        .register_relay(RelayRegistration {
            relay_id: config.relay_id.clone(),
            region: config.region.clone(),
            public_base_url: config.public_base_url.clone(),
            internal_base_url: config.internal_base_url.clone(),
            max_active_tunnels: config.max_active_tunnels,
        })
        .await
        .context("registering relay node")?;

    let tunnels = Arc::new(RwLock::new(HashMap::new()));
    send_relay_heartbeat(&store, &config.relay_id, tunnels.clone())
        .await
        .context("recording initial relay heartbeat")?;
    tokio::spawn(relay_heartbeat_loop(
        store.clone(),
        config.relay_id.clone(),
        tunnels.clone(),
    ));

    let state = RelayState {
        tunnels,
        master_secret,
        store,
        relay_id: config.relay_id,
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/connect/:tunnel_id", get(desktop_connect_ws))
        .route(
            "/t/:tunnel_id/*path",
            get(http_proxy::proxy_get)
                .post(http_proxy::proxy_http)
                .put(http_proxy::proxy_http)
                .delete(http_proxy::proxy_http)
                .patch(http_proxy::proxy_http)
                .options(http_proxy::proxy_http)
                .head(http_proxy::proxy_http),
        )
        .with_state(state);

    let addr: SocketAddr = args.listen.parse().context("parsing --listen")?;
    info!("listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("binding listener")?;
    axum::serve(listener, app).await.context("serving relay")?;
    Ok(())
}

async fn health(State(state): State<RelayState>) -> Response {
    match state.store.health_snapshot().await {
        Ok(snapshot) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "ok": true,
                "relay_count": snapshot.relay_count,
                "healthy_relay_count": snapshot.healthy_relay_count,
            })),
        )
            .into_response(),
        Err(err) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "ok": false,
                "error": err.to_string(),
            })),
        )
            .into_response(),
    }
}

async fn desktop_connect_ws(
    State(state): State<RelayState>,
    Path(tunnel_id): Path<String>,
    headers: HeaderMap,
    ws: axum::extract::ws::WebSocketUpgrade,
) -> impl IntoResponse {
    if let Err(status) = validate_tunnel_for_relay(&state, &tunnel_id).await {
        return status.into_response();
    }
    let secret = match extract_desktop_secret(&headers) {
        Ok(secret) => secret,
        Err(status) => return status.into_response(),
    };

    ws.on_upgrade(move |socket| async move {
        if let Err(err) = handle_desktop_socket(state, tunnel_id, secret, socket).await {
            warn!("desktop ws ended with error: {err:#}");
        }
    })
    .into_response()
}

fn extract_desktop_secret(headers: &HeaderMap) -> std::result::Result<String, StatusCode> {
    let Some(value) = headers.get(TUNNEL_SECRET_HEADER) else {
        return Err(StatusCode::UNAUTHORIZED);
    };
    let value = value.to_str().map_err(|_| StatusCode::UNAUTHORIZED)?;
    if value.is_empty() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(value.to_string())
}

async fn handle_desktop_socket(
    state: RelayState,
    tunnel_id: String,
    secret: String,
    socket: WebSocket,
) -> Result<()> {
    let tunnel = get_or_create_tunnel(&state, &tunnel_id).await;

    {
        let mut inner = tunnel.inner.lock().await;
        let expected = match derive_secret(&state.master_secret, &tunnel_id) {
            Ok(value) => value,
            Err(err) => {
                warn!(
                    "rejecting desktop connect for tunnel {tunnel_id}: unable to derive secret: {err:#}"
                );
                return Ok(());
            }
        };
        if secret.as_bytes().ct_eq(expected.as_bytes()).unwrap_u8() != 1 {
            warn!("rejecting desktop connect for tunnel {tunnel_id}: secret mismatch");
            return Ok(());
        }

        // Replace any existing desktop connection.
        inner.desktop = None;
        inner.pending_http.clear();
        inner.pending_ws_open.clear();
    }
    state
        .store
        .record_desktop_connected(&tunnel_id, &state.relay_id)
        .await
        .context("recording desktop tunnel connection")?;

    let (mut ws_tx, mut ws_rx) = socket.split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<RelayToClient>();
    {
        let mut inner = tunnel.inner.lock().await;
        inner.desktop = Some(DesktopHandle { tx: out_tx.clone() });
    }

    let tunnel_for_read = tunnel.clone();
    let tunnel_for_write = tunnel.clone();

    let write_task = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            let text = match serde_json::to_string(&msg) {
                Ok(t) => t,
                Err(err) => {
                    warn!("failed to serialize relay message: {err}");
                    continue;
                }
            };
            if ws_tx.send(Message::Text(text)).await.is_err() {
                break;
            }
        }
    });

    let read_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_rx.next().await {
            match msg {
                Message::Text(text) => match serde_json::from_str::<ClientToRelay>(&text) {
                    Ok(parsed) => dispatch_client_message(&tunnel_for_read, parsed).await,
                    Err(err) => warn!("invalid client message: {err}"),
                },
                Message::Close(_) => break,
                _ => {}
            }
        }
    });

    tokio::select! {
        _ = write_task => {},
        _ = read_task => {},
    }

    {
        let mut inner = tunnel_for_write.inner.lock().await;
        inner.desktop = None;
        inner.pending_http.clear();
        inner.pending_ws_open.clear();
        inner.ws_streams.clear();
    }
    if let Err(err) = state
        .store
        .record_desktop_disconnected(&tunnel_id, &state.relay_id)
        .await
    {
        warn!("failed to record desktop tunnel disconnect: {err}");
    }

    Ok(())
}

async fn dispatch_client_message(tunnel: &Arc<Tunnel>, msg: ClientToRelay) {
    match msg {
        ClientToRelay::HttpResponse {
            id,
            status,
            headers,
            body_b64,
        } => {
            let body = BASE64.decode(body_b64.as_bytes()).unwrap_or_default();
            let sender = {
                let mut inner = tunnel.inner.lock().await;
                inner.pending_http.remove(&id)
            };
            if let Some(tx) = sender {
                let _ = tx.send(HttpResponse {
                    status,
                    headers,
                    body,
                });
            }
        }
        ClientToRelay::WsOpenResult { id, ok, error } => {
            let sender = {
                let mut inner = tunnel.inner.lock().await;
                inner.pending_ws_open.remove(&id)
            };
            if let Some(tx) = sender {
                let res = if ok {
                    Ok(())
                } else {
                    Err(error.unwrap_or_else(|| "ws open failed".to_string()))
                };
                let _ = tx.send(res);
            }
        }
        ClientToRelay::WsMessage {
            id,
            is_binary,
            data,
        } => {
            let handle = {
                let inner = tunnel.inner.lock().await;
                inner.ws_streams.get(&id).cloned()
            };
            let Some(handle) = handle else {
                return;
            };
            let msg = if is_binary {
                let bytes = BASE64.decode(data.as_bytes()).unwrap_or_default();
                Message::Binary(bytes)
            } else {
                Message::Text(data)
            };
            let _ = handle.mobile_tx.send(msg);
        }
        ClientToRelay::WsClosed { id, code, reason } => {
            let handle = {
                let mut inner = tunnel.inner.lock().await;
                inner.ws_streams.remove(&id)
            };
            let Some(handle) = handle else {
                return;
            };
            let close = axum::extract::ws::CloseFrame {
                code: axum::extract::ws::close_code::NORMAL,
                reason: reason.unwrap_or_default().into(),
            };
            let _ = handle.mobile_tx.send(Message::Close(Some(close)));
            if let Some(_code) = code {
                // ignore for now
            }
        }
    }
}

async fn handle_mobile_ws(
    state: RelayState,
    tunnel_id: String,
    path: String,
    query: Option<String>,
    headers: HeaderMap,
    mut socket: WebSocket,
) -> Result<()> {
    validate_tunnel_for_relay(&state, &tunnel_id)
        .await
        .map_err(|status| anyhow!("relay rejected tunnel assignment with status {status}"))?;
    let tunnel = get_or_create_tunnel(&state, &tunnel_id).await;

    let desktop = {
        let inner = tunnel.inner.lock().await;
        inner.desktop.clone()
    }
    .context("tunnel not connected")?;

    let stream_id = Uuid::new_v4().to_string();

    let (open_tx, open_rx) = oneshot::channel::<Result<(), String>>();
    {
        let mut inner = tunnel.inner.lock().await;
        inner.pending_ws_open.insert(stream_id.clone(), open_tx);
    }

    let forward_headers = extract_forward_headers(&headers);
    let mut full_path = format!("/{}", path.trim_start_matches('/'));
    if let Some(q) = query {
        if !q.is_empty() {
            full_path.push('?');
            full_path.push_str(&q);
        }
    }

    desktop
        .tx
        .send(RelayToClient::WsOpen {
            id: stream_id.clone(),
            path: full_path,
            headers: forward_headers,
        })
        .context("sending ws open")?;

    match tokio::time::timeout(Duration::from_secs(15), open_rx).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(e))) => {
            let _ = socket
                .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                    code: axum::extract::ws::close_code::ERROR,
                    reason: e.into(),
                })))
                .await;
            return Ok(());
        }
        _ => {
            let _ = socket.send(Message::Close(None)).await;
            return Ok(());
        }
    }

    let (mobile_tx, mut mobile_rx) = mpsc::unbounded_channel::<Message>();
    {
        let mut inner = tunnel.inner.lock().await;
        inner
            .ws_streams
            .insert(stream_id.clone(), WsStreamHandle { mobile_tx });
    }

    let (mut ws_tx, mut ws_rx) = socket.split();
    let send_task = tokio::spawn(async move {
        while let Some(msg) = mobile_rx.recv().await {
            if ws_tx.send(msg).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(msg)) = ws_rx.next().await {
        match msg {
            Message::Text(text) => {
                let _ = desktop.tx.send(RelayToClient::WsMessage {
                    id: stream_id.clone(),
                    is_binary: false,
                    data: text,
                });
            }
            Message::Binary(bytes) => {
                let _ = desktop.tx.send(RelayToClient::WsMessage {
                    id: stream_id.clone(),
                    is_binary: true,
                    data: BASE64.encode(bytes),
                });
            }
            Message::Close(frame) => {
                let (code, reason) = frame
                    .map(|f| (Some(f.code), Some(f.reason.to_string())))
                    .unwrap_or((None, None));
                let _ = desktop.tx.send(RelayToClient::WsClose {
                    id: stream_id.clone(),
                    code,
                    reason,
                });
                break;
            }
            _ => {}
        }
    }

    send_task.abort();
    {
        let mut inner = tunnel.inner.lock().await;
        inner.ws_streams.remove(&stream_id);
        inner.pending_ws_open.remove(&stream_id);
    }

    let _ = desktop.tx.send(RelayToClient::WsClose {
        id: stream_id,
        code: None,
        reason: None,
    });

    Ok(())
}

async fn get_or_create_tunnel(state: &RelayState, tunnel_id: &str) -> Arc<Tunnel> {
    {
        let map = state.tunnels.read().await;
        if let Some(t) = map.get(tunnel_id) {
            return t.clone();
        }
    }
    let mut map = state.tunnels.write().await;
    map.entry(tunnel_id.to_string())
        .or_insert_with(|| {
            Arc::new(Tunnel {
                inner: Mutex::new(TunnelInner {
                    desktop: None,
                    pending_http: HashMap::new(),
                    pending_ws_open: HashMap::new(),
                    ws_streams: HashMap::new(),
                }),
            })
        })
        .clone()
}

fn load_master_secret() -> Result<Vec<u8>> {
    let raw = std::env::var("CTX_TUNNEL_MASTER_SECRET")
        .context("CTX_TUNNEL_MASTER_SECRET must be set")?;
    parse_master_secret(&raw)
}

struct RelayConfig {
    database_url: String,
    relay_id: String,
    region: String,
    public_base_url: String,
    internal_base_url: String,
    max_active_tunnels: i32,
}

fn load_relay_config() -> Result<RelayConfig> {
    let max_active_tunnels = std::env::var("CTX_TUNNEL_RELAY_MAX_ACTIVE_TUNNELS")
        .context("CTX_TUNNEL_RELAY_MAX_ACTIVE_TUNNELS must be set")?
        .parse::<i32>()
        .context("CTX_TUNNEL_RELAY_MAX_ACTIVE_TUNNELS must be an integer")?;
    if max_active_tunnels <= 0 {
        anyhow::bail!("CTX_TUNNEL_RELAY_MAX_ACTIVE_TUNNELS must be positive");
    }
    Ok(RelayConfig {
        database_url: std::env::var("MOBILE_TUNNEL_DATABASE_URL")
            .context("MOBILE_TUNNEL_DATABASE_URL must be set")?,
        relay_id: std::env::var("CTX_TUNNEL_RELAY_ID")
            .context("CTX_TUNNEL_RELAY_ID must be set")?,
        region: std::env::var("CTX_TUNNEL_RELAY_REGION")
            .context("CTX_TUNNEL_RELAY_REGION must be set")?,
        public_base_url: std::env::var("CTX_TUNNEL_RELAY_PUBLIC_BASE_URL")
            .context("CTX_TUNNEL_RELAY_PUBLIC_BASE_URL must be set")?,
        internal_base_url: std::env::var("CTX_TUNNEL_RELAY_INTERNAL_BASE_URL")
            .context("CTX_TUNNEL_RELAY_INTERNAL_BASE_URL must be set")?,
        max_active_tunnels,
    })
}

async fn validate_tunnel_for_relay(
    state: &RelayState,
    tunnel_id: &str,
) -> std::result::Result<(), StatusCode> {
    match state
        .store
        .validate_tunnel_relay_assignment(tunnel_id, &state.relay_id)
        .await
    {
        Ok(RelayAssignmentValidation::Valid) => Ok(()),
        Ok(
            RelayAssignmentValidation::UnknownTunnel
            | RelayAssignmentValidation::TunnelDisabled
            | RelayAssignmentValidation::WrongRelay { .. },
        ) => Err(StatusCode::NOT_FOUND),
        Err(err) => {
            warn!("failed to validate relay assignment: {err}");
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}

async fn relay_heartbeat_loop(
    store: TunnelStore,
    relay_id: String,
    tunnels: Arc<RwLock<HashMap<String, Arc<Tunnel>>>>,
) {
    let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
    loop {
        interval.tick().await;
        if let Err(err) = send_relay_heartbeat(&store, &relay_id, tunnels.clone()).await {
            warn!("failed to record relay heartbeat: {err}");
        }
    }
}

async fn send_relay_heartbeat(
    store: &TunnelStore,
    relay_id: &str,
    tunnels: Arc<RwLock<HashMap<String, Arc<Tunnel>>>>,
) -> Result<()> {
    let count = active_connected_tunnel_count(tunnels).await;
    store
        .heartbeat_relay(RelayHeartbeat {
            relay_id: relay_id.to_string(),
            active_tunnel_count: count,
            observed_at: Utc::now(),
        })
        .await
        .context("recording relay heartbeat")
}

async fn active_connected_tunnel_count(tunnels: Arc<RwLock<HashMap<String, Arc<Tunnel>>>>) -> i32 {
    let snapshot = {
        let map = tunnels.read().await;
        map.values().cloned().collect::<Vec<_>>()
    };
    let mut count: i32 = 0;
    for tunnel in snapshot {
        if tunnel.inner.lock().await.desktop.is_some() && count < i32::MAX {
            count += 1;
        }
    }
    count
}

fn parse_master_secret(raw: &str) -> Result<Vec<u8>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("CTX_TUNNEL_MASTER_SECRET must not be empty"));
    }
    Ok(trimmed.as_bytes().to_vec())
}

fn derive_secret(master_secret: &[u8], tunnel_id: &str) -> Result<String> {
    let mut mac = Hmac::<Sha256>::new_from_slice(master_secret)
        .map_err(|err| anyhow!("failed to initialize hmac for tunnel secret derivation: {err}"))?;
    mac.update(tunnel_id.as_bytes());
    let bytes = mac.finalize().into_bytes();
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

static BASE64: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::STANDARD;

#[cfg(test)]
mod tests;
