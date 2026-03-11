use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::{HeaderMap, Method, Request, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use base64::Engine;
use clap::Parser;
use futures::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(name = "ctx-tunnel-relay", version)]
struct Args {
    #[arg(long, default_value = "127.0.0.1:8787")]
    listen: String,
}

#[derive(Clone)]
struct RelayState {
    tunnels: Arc<RwLock<HashMap<String, Arc<Tunnel>>>>,
    master_secret: Option<Vec<u8>>,
}

struct Tunnel {
    inner: Mutex<TunnelInner>,
}

struct TunnelInner {
    secret: Option<String>,
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

#[derive(Debug, Deserialize)]
struct ConnectQuery {
    secret: String,
}

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
    let master_secret = std::env::var("CTX_TUNNEL_MASTER_SECRET")
        .ok()
        .map(|s| s.into_bytes());
    let state = RelayState {
        tunnels: Arc::new(RwLock::new(HashMap::new())),
        master_secret,
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/connect/:tunnel_id", get(desktop_connect_ws))
        .route(
            "/t/:tunnel_id/*path",
            get(proxy_get)
                .post(proxy_http)
                .put(proxy_http)
                .delete(proxy_http)
                .patch(proxy_http)
                .options(proxy_http)
                .head(proxy_http),
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

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "ok": true }))
}

async fn desktop_connect_ws(
    State(state): State<RelayState>,
    Path(tunnel_id): Path<String>,
    Query(q): Query<ConnectQuery>,
    ws: axum::extract::ws::WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        if let Err(err) = handle_desktop_socket(state, tunnel_id, q.secret, socket).await {
            warn!("desktop ws ended with error: {err:#}");
        }
    })
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
        if let Some(master) = state.master_secret.as_ref() {
            let expected = match derive_secret(master, &tunnel_id) {
                Ok(value) => value,
                Err(err) => {
                    warn!("rejecting desktop connect for tunnel {tunnel_id}: unable to derive secret: {err:#}");
                    return Ok(());
                }
            };
            if secret.as_bytes().ct_eq(expected.as_bytes()).unwrap_u8() != 1 {
                warn!("rejecting desktop connect for tunnel {tunnel_id}: secret mismatch");
                return Ok(());
            }
            inner.secret = Some(expected);
        } else if let Some(existing) = inner.secret.as_deref() {
            if existing != secret {
                warn!("rejecting desktop connect for tunnel {tunnel_id}: secret mismatch");
                return Ok(());
            }
        } else {
            inner.secret = Some(secret);
        }

        // Replace any existing desktop connection.
        inner.desktop = None;
        inner.pending_http.clear();
        inner.pending_ws_open.clear();
    }

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

async fn proxy_get(
    State(state): State<RelayState>,
    Path((tunnel_id, path)): Path<(String, String)>,
    req: Request<axum::body::Body>,
) -> Response {
    let (mut parts, body) = req.into_parts();
    let is_ws = is_websocket_upgrade(&parts.headers);
    if is_ws {
        match axum::extract::ws::WebSocketUpgrade::from_request_parts(&mut parts, &state).await {
            Ok(ws) => {
                let headers = parts.headers.clone();
                let uri = parts.uri.clone();
                return ws
                    .on_upgrade(move |socket| async move {
                        if let Err(err) = handle_mobile_ws(
                            state,
                            tunnel_id,
                            path,
                            uri.query().map(str::to_string),
                            headers,
                            socket,
                        )
                        .await
                        {
                            warn!("mobile ws error: {err:#}");
                        }
                    })
                    .into_response();
            }
            Err(_) => return StatusCode::BAD_REQUEST.into_response(),
        }
    }

    let req = Request::from_parts(parts, body);
    proxy_http_inner(state, tunnel_id, path, req).await
}

async fn proxy_http(
    State(state): State<RelayState>,
    Path((tunnel_id, path)): Path<(String, String)>,
    uri: Uri,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let uri_path = format!("/{}", path.trim_start_matches('/'));
    proxy_http_message(
        state,
        tunnel_id,
        method,
        uri_path,
        uri.query().map(str::to_string),
        headers,
        body,
    )
    .await
}

async fn proxy_http_inner(
    state: RelayState,
    tunnel_id: String,
    path: String,
    req: Request<axum::body::Body>,
) -> Response {
    let (parts, body) = req.into_parts();
    let method = parts.method;
    let headers = parts.headers;
    let query = parts.uri.query().map(str::to_string);
    let bytes = match axum::body::to_bytes(body, 16 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let uri = format!("/{}", path.trim_start_matches('/'));
    proxy_http_message(state, tunnel_id, method, uri, query, headers, bytes).await
}

async fn proxy_http_message(
    state: RelayState,
    tunnel_id: String,
    method: Method,
    path: String,
    query: Option<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let tunnel = get_or_create_tunnel(&state, &tunnel_id).await;

    let desktop = {
        let inner = tunnel.inner.lock().await;
        inner.desktop.clone()
    };
    let Some(desktop) = desktop else {
        return (StatusCode::BAD_GATEWAY, "tunnel not connected").into_response();
    };

    let request_id = Uuid::new_v4().to_string();
    let (tx, rx) = oneshot::channel::<HttpResponse>();
    {
        let mut inner = tunnel.inner.lock().await;
        inner.pending_http.insert(request_id.clone(), tx);
    }

    let forward_headers = extract_ws_forward_headers(&headers);
    let mut full_path = path;
    if let Some(q) = query {
        if !q.is_empty() {
            full_path.push('?');
            full_path.push_str(&q);
        }
    }

    let msg = RelayToClient::HttpRequest {
        id: request_id.clone(),
        method: method.to_string(),
        path: full_path,
        headers: forward_headers,
        body_b64: BASE64.encode(body),
    };
    if desktop.tx.send(msg).is_err() {
        return (StatusCode::BAD_GATEWAY, "tunnel disconnected").into_response();
    }

    match tokio::time::timeout(Duration::from_secs(30), rx).await {
        Ok(Ok(resp)) => build_http_response(resp),
        Ok(Err(_)) => (StatusCode::BAD_GATEWAY, "tunnel response dropped").into_response(),
        Err(_) => (StatusCode::GATEWAY_TIMEOUT, "tunnel request timed out").into_response(),
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
                    secret: None,
                    desktop: None,
                    pending_http: HashMap::new(),
                    pending_ws_open: HashMap::new(),
                    ws_streams: HashMap::new(),
                }),
            })
        })
        .clone()
}

fn is_websocket_upgrade(headers: &HeaderMap) -> bool {
    let Some(upgrade) = headers.get(axum::http::header::UPGRADE) else {
        return false;
    };
    let Ok(upgrade) = upgrade.to_str() else {
        return false;
    };
    upgrade.eq_ignore_ascii_case("websocket")
}

fn extract_forward_headers(headers: &HeaderMap) -> Vec<(String, String)> {
    const HOP_BY_HOP: &[axum::http::header::HeaderName] = &[
        axum::http::header::CONNECTION,
        axum::http::header::UPGRADE,
        axum::http::header::PROXY_AUTHENTICATE,
        axum::http::header::PROXY_AUTHORIZATION,
        axum::http::header::TE,
        axum::http::header::TRAILER,
        axum::http::header::TRANSFER_ENCODING,
        axum::http::header::HOST,
    ];

    headers
        .iter()
        .filter(|(k, _)| !HOP_BY_HOP.contains(k))
        .filter_map(|(k, v)| Some((k.to_string(), v.to_str().ok()?.to_string())))
        .collect()
}

fn extract_ws_forward_headers(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .filter(|(k, _)| k.as_str().eq_ignore_ascii_case("authorization"))
        .filter_map(|(k, v)| Some((k.to_string(), v.to_str().ok()?.to_string())))
        .collect()
}

fn build_http_response(resp: HttpResponse) -> Response {
    let status = StatusCode::from_u16(resp.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut builder = Response::builder().status(status);
    if let Some(headers) = builder.headers_mut() {
        for (k, v) in resp.headers {
            if let (Ok(name), Ok(value)) = (
                axum::http::header::HeaderName::from_bytes(k.as_bytes()),
                axum::http::HeaderValue::from_str(&v),
            ) {
                headers.insert(name, value);
            }
        }
    } else {
        tracing::warn!(
            "response builder headers unavailable; returning response without forwarded headers"
        );
    }
    builder
        .body(axum::body::Body::from(resp.body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
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
mod tests {
    use super::*;

    fn test_tunnel() -> Arc<Tunnel> {
        Arc::new(Tunnel {
            inner: Mutex::new(TunnelInner {
                secret: None,
                desktop: None,
                pending_http: HashMap::new(),
                pending_ws_open: HashMap::new(),
                ws_streams: HashMap::new(),
            }),
        })
    }

    #[tokio::test]
    async fn dispatch_client_message_delivers_pending_http_response() {
        let tunnel = test_tunnel();
        let (tx, rx) = oneshot::channel();
        {
            let mut inner = tunnel.inner.lock().await;
            inner.pending_http.insert("req-1".to_string(), tx);
        }

        dispatch_client_message(
            &tunnel,
            ClientToRelay::HttpResponse {
                id: "req-1".to_string(),
                status: 204,
                headers: vec![("content-type".to_string(), "text/plain".to_string())],
                body_b64: BASE64.encode("ok"),
            },
        )
        .await;

        let response = rx.await.expect("http response");
        assert_eq!(response.status, 204);
        assert_eq!(response.body, b"ok");
        assert!(tunnel.inner.lock().await.pending_http.is_empty());
    }

    #[tokio::test]
    async fn dispatch_client_message_delivers_ws_open_result_and_forwards_messages() {
        let tunnel = test_tunnel();
        let (open_tx, open_rx) = oneshot::channel();
        let (mobile_tx, mut mobile_rx) = mpsc::unbounded_channel();
        {
            let mut inner = tunnel.inner.lock().await;
            inner
                .pending_ws_open
                .insert("stream-1".to_string(), open_tx);
            inner
                .ws_streams
                .insert("stream-1".to_string(), WsStreamHandle { mobile_tx });
        }

        dispatch_client_message(
            &tunnel,
            ClientToRelay::WsOpenResult {
                id: "stream-1".to_string(),
                ok: true,
                error: None,
            },
        )
        .await;
        assert!(open_rx.await.expect("ws open result").is_ok());

        dispatch_client_message(
            &tunnel,
            ClientToRelay::WsMessage {
                id: "stream-1".to_string(),
                is_binary: false,
                data: "hello".to_string(),
            },
        )
        .await;
        let forwarded = mobile_rx.recv().await.expect("forwarded mobile message");
        assert_eq!(forwarded, Message::Text("hello".to_string()));
    }

    #[tokio::test]
    async fn dispatch_client_message_ignores_unknown_ids_and_removes_closed_streams() {
        let tunnel = test_tunnel();
        let (mobile_tx, mut mobile_rx) = mpsc::unbounded_channel();
        {
            let mut inner = tunnel.inner.lock().await;
            inner
                .ws_streams
                .insert("stream-1".to_string(), WsStreamHandle { mobile_tx });
        }

        dispatch_client_message(
            &tunnel,
            ClientToRelay::WsMessage {
                id: "missing".to_string(),
                is_binary: false,
                data: "ignored".to_string(),
            },
        )
        .await;
        assert!(mobile_rx.try_recv().is_err());

        dispatch_client_message(
            &tunnel,
            ClientToRelay::WsClosed {
                id: "stream-1".to_string(),
                code: Some(1000),
                reason: Some("done".to_string()),
            },
        )
        .await;
        let closed = mobile_rx.recv().await.expect("close forwarded");
        assert!(matches!(closed, Message::Close(_)));
        assert!(tunnel.inner.lock().await.ws_streams.is_empty());
    }

    #[test]
    fn relay_header_helpers_strip_hop_by_hop_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(axum::http::header::HOST, "example.test".parse().unwrap());
        headers.insert(axum::http::header::CONNECTION, "upgrade".parse().unwrap());
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer secret".parse().unwrap(),
        );
        headers.insert("x-custom", "value".parse().unwrap());

        let forwarded = extract_forward_headers(&headers);
        assert!(forwarded.contains(&(String::from("authorization"), String::from("Bearer secret"))));
        assert!(forwarded.contains(&(String::from("x-custom"), String::from("value"))));
        assert!(!forwarded
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("host")));
        assert!(!forwarded
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("connection")));

        let ws_forwarded = extract_ws_forward_headers(&headers);
        assert_eq!(
            ws_forwarded,
            vec![(String::from("authorization"), String::from("Bearer secret"))]
        );
    }

    #[test]
    fn derive_secret_is_deterministic() {
        let first = derive_secret(b"master-secret", "tunnel-1").unwrap();
        let second = derive_secret(b"master-secret", "tunnel-1").unwrap();
        let third = derive_secret(b"master-secret", "tunnel-2").unwrap();
        assert_eq!(first, second);
        assert_ne!(first, third);
    }
}
