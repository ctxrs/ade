use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use axum::body::{Body, Bytes};
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{FromRequestParts, Path, State};
use axum::http::{HeaderMap, Method, Request, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use clap::Parser;
use futures::{SinkExt, StreamExt};
use redis::aio::ConnectionManager;
use serde::{Deserialize, Serialize};
use sqlx::{postgres::PgPoolOptions, Pool, Postgres, Row};
use tokio::sync::RwLock;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message as TungsteniteMessage;
use tracing::{info, warn};
use url::Url;

#[derive(Parser, Debug)]
#[command(name = "ctx-tunnel-router", version)]
struct Args {
    #[arg(long, default_value = "0.0.0.0:8790")]
    listen: String,
    #[arg(long, default_value = "30")]
    cache_ttl_secs: u64,
}

#[derive(Clone)]
struct RouterState {
    store: Arc<TunnelStore>,
    client: reqwest::Client,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TunnelTarget {
    relay_base_url: String,
    public_base_url: String,
}

struct CachedTarget {
    target: TunnelTarget,
    expires_at: Instant,
}

struct TunnelStore {
    db: Pool<Postgres>,
    redis: Option<ConnectionManager>,
    cache: RwLock<HashMap<String, CachedTarget>>,
    cache_ttl: Duration,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let database_url = std::env::var("CONTROL_PLANE_DATABASE_URL")
        .context("missing CONTROL_PLANE_DATABASE_URL")?;
    let redis_url = std::env::var("CONTROL_PLANE_REDIS_URL").ok();

    let db = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await
        .context("connecting to postgres")?;

    let redis = match redis_url {
        Some(url) if !url.trim().is_empty() => {
            let client = redis::Client::open(url)?;
            Some(ConnectionManager::new(client).await?)
        }
        _ => None,
    };

    let store = Arc::new(TunnelStore {
        db,
        redis,
        cache: RwLock::new(HashMap::new()),
        cache_ttl: Duration::from_secs(args.cache_ttl_secs),
    });

    let state = RouterState {
        store,
        client: reqwest::Client::new(),
    };

    let app = Router::new()
        .route("/health", get(health))
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
    info!("router listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("binding listener")?;
    axum::serve(listener, app).await.context("serving router")?;
    Ok(())
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "ok": true }))
}

async fn proxy_get(
    State(state): State<RouterState>,
    Path((tunnel_id, path)): Path<(String, String)>,
    req: Request<Body>,
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
                        if let Err(err) = handle_ws_proxy(
                            state,
                            tunnel_id,
                            path,
                            uri.query().map(str::to_string),
                            headers,
                            socket,
                        )
                        .await
                        {
                            warn!("ws proxy error: {err:#}");
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
    State(state): State<RouterState>,
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
        uri_path,
        method,
        uri.query().map(str::to_string),
        headers,
        body,
    )
    .await
}

async fn proxy_http_inner(
    state: RouterState,
    tunnel_id: String,
    path: String,
    req: Request<Body>,
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
    proxy_http_message(state, tunnel_id, uri, method, query, headers, bytes).await
}

async fn proxy_http_message(
    state: RouterState,
    tunnel_id: String,
    path: String,
    method: Method,
    query: Option<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(target) = state.store.resolve_target(&tunnel_id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let upstream_url =
        build_upstream_url(&target.relay_base_url, &tunnel_id, &path, query.as_deref());
    let mut req = state.client.request(method, upstream_url).body(body);

    for (name, value) in headers.iter() {
        if name == axum::http::header::HOST || name == axum::http::header::CONTENT_LENGTH {
            continue;
        }
        if let Ok(val) = value.to_str() {
            req = req.header(name, val);
        }
    }

    let resp = match req.send().await {
        Ok(resp) => resp,
        Err(err) => {
            warn!("upstream request failed: {err}");
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };

    let status = resp.status();
    let mut builder = Response::builder().status(status);
    for (key, value) in resp.headers() {
        if let Ok(val) = value.to_str() {
            builder = builder.header(key, val);
        }
    }
    let body = match resp.bytes().await {
        Ok(bytes) => bytes,
        Err(_) => Bytes::new(),
    };
    builder
        .body(Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

async fn handle_ws_proxy(
    state: RouterState,
    tunnel_id: String,
    path: String,
    query: Option<String>,
    headers: HeaderMap,
    socket: WebSocket,
) -> Result<()> {
    let Some(target) = state.store.resolve_target(&tunnel_id).await else {
        return Ok(());
    };

    let upstream = build_upstream_url(&target.relay_base_url, &tunnel_id, &path, query.as_deref());
    let ws_url = to_ws_url(&upstream)?;

    let mut req = ws_url.as_str().into_client_request()?;
    let forward = extract_ws_forward_headers(&headers);
    for (key, value) in forward {
        req.headers_mut().insert(key, value);
    }

    let (upstream_ws, _) = tokio_tungstenite::connect_async(req).await?;
    let (mut upstream_tx, mut upstream_rx) = upstream_ws.split();
    let (mut downstream_tx, mut downstream_rx) = socket.split();

    let upstream_to_downstream = tokio::spawn(async move {
        while let Some(msg) = upstream_rx.next().await {
            let Ok(msg) = msg else { break };
            let out = match msg {
                TungsteniteMessage::Text(text) => Message::Text(text.to_string()),
                TungsteniteMessage::Binary(data) => Message::Binary(data.to_vec()),
                TungsteniteMessage::Close(frame) => {
                    Message::Close(frame.map(|f| axum::extract::ws::CloseFrame {
                        code: f.code.into(),
                        reason: f.reason.to_string().into(),
                    }))
                }
                TungsteniteMessage::Ping(data) => Message::Ping(data.to_vec()),
                TungsteniteMessage::Pong(data) => Message::Pong(data.to_vec()),
                TungsteniteMessage::Frame(_) => continue,
            };
            if downstream_tx.send(out).await.is_err() {
                break;
            }
        }
    });

    let downstream_to_upstream = tokio::spawn(async move {
        while let Some(msg) = downstream_rx.next().await {
            let Ok(msg) = msg else { break };
            let out = match msg {
                Message::Text(text) => TungsteniteMessage::Text(text.into()),
                Message::Binary(data) => TungsteniteMessage::Binary(data.into()),
                Message::Close(frame) => TungsteniteMessage::Close(frame.map(|f| {
                    tokio_tungstenite::tungstenite::protocol::CloseFrame {
                        code: f.code.into(),
                        reason: f.reason.to_string().into(),
                    }
                })),
                Message::Ping(data) => TungsteniteMessage::Ping(data.into()),
                Message::Pong(data) => TungsteniteMessage::Pong(data.into()),
            };
            if upstream_tx.send(out).await.is_err() {
                break;
            }
        }
    });

    let _ = tokio::join!(upstream_to_downstream, downstream_to_upstream);
    Ok(())
}

fn build_upstream_url(base: &str, tunnel_id: &str, path: &str, query: Option<&str>) -> String {
    let trimmed = base.trim_end_matches('/');
    let mut url = format!("{trimmed}/t/{tunnel_id}/{}", path.trim_start_matches('/'));
    if let Some(q) = query {
        url.push('?');
        url.push_str(q);
    }
    url
}

fn to_ws_url(http_url: &str) -> Result<Url> {
    let mut url = Url::parse(http_url)?;
    if url.scheme() == "https" {
        let _ = url.set_scheme("wss");
    } else if url.scheme() == "http" {
        let _ = url.set_scheme("ws");
    }
    Ok(url)
}

fn is_websocket_upgrade(headers: &HeaderMap) -> bool {
    let Some(upgrade) = headers.get(axum::http::header::UPGRADE) else {
        return false;
    };
    upgrade
        .to_str()
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false)
}

fn extract_ws_forward_headers(
    headers: &HeaderMap,
) -> Vec<(axum::http::HeaderName, axum::http::HeaderValue)> {
    headers
        .iter()
        .filter(|(k, _)| k.as_str().eq_ignore_ascii_case("authorization"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

impl TunnelStore {
    async fn resolve_target(&self, tunnel_id: &str) -> Option<TunnelTarget> {
        if let Some(entry) = self.get_cached(tunnel_id).await {
            return Some(entry);
        }
        if let Some(target) = self.get_redis(tunnel_id).await {
            self.set_cache(tunnel_id, &target).await;
            return Some(target);
        }
        let row = sqlx::query(
            r#"SELECT relay_base_url, public_base_url, disabled_at IS NOT NULL AS disabled
               FROM mobile_tunnels
               WHERE tunnel_id = $1"#,
        )
        .bind(tunnel_id)
        .fetch_optional(&self.db)
        .await
        .ok()??;

        let disabled: bool = row.try_get("disabled").ok()?;
        if disabled {
            return None;
        }

        let relay_base_url: String = row.try_get("relay_base_url").ok()?;
        let public_base_url: String = row.try_get("public_base_url").ok()?;
        let target = TunnelTarget {
            relay_base_url,
            public_base_url,
        };
        self.set_cache(tunnel_id, &target).await;
        self.set_redis(tunnel_id, &target).await;
        Some(target)
    }

    async fn get_cached(&self, tunnel_id: &str) -> Option<TunnelTarget> {
        let now = Instant::now();
        let cache = self.cache.read().await;
        cache.get(tunnel_id).and_then(|entry| {
            if entry.expires_at > now {
                Some(entry.target.clone())
            } else {
                None
            }
        })
    }

    async fn set_cache(&self, tunnel_id: &str, target: &TunnelTarget) {
        let expires_at = Instant::now() + self.cache_ttl;
        let mut cache = self.cache.write().await;
        cache.insert(
            tunnel_id.to_string(),
            CachedTarget {
                target: target.clone(),
                expires_at,
            },
        );
    }

    async fn get_redis(&self, tunnel_id: &str) -> Option<TunnelTarget> {
        let mut conn = self.redis.clone()?;
        let key = format!("tunnel:{tunnel_id}");
        let data: Option<String> = redis::AsyncCommands::get(&mut conn, key).await.ok();
        let json = data?;
        serde_json::from_str(&json).ok()
    }

    async fn set_redis(&self, tunnel_id: &str, target: &TunnelTarget) {
        let Some(mut conn) = self.redis.clone() else {
            return;
        };
        let Ok(json) = serde_json::to_string(target) else {
            return;
        };
        let key = format!("tunnel:{tunnel_id}");
        let _: redis::RedisResult<()> =
            redis::AsyncCommands::set_ex(&mut conn, key, json, self.cache_ttl.as_secs()).await;
    }
}
