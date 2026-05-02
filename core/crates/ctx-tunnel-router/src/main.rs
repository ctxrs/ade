use std::net::SocketAddr;

use anyhow::{Context, Result};
use axum::body::{Body, Bytes};
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{FromRequestParts, Path, State};
use axum::http::{HeaderMap, Method, Request, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use clap::Parser;
use ctx_tunnel_store::{ResolvedTunnelTarget, TunnelResolveResult, TunnelStore};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message as TungsteniteMessage;
use tracing::{info, warn};
use url::Url;

#[derive(Parser, Debug)]
#[command(name = "ctx-tunnel-router", version)]
struct Args {
    #[arg(long, default_value = "0.0.0.0:8790")]
    listen: String,
}

#[derive(Clone)]
struct RouterState {
    store: TunnelStore,
    client: reqwest::Client,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let database_url = std::env::var("MOBILE_TUNNEL_DATABASE_URL")
        .context("missing MOBILE_TUNNEL_DATABASE_URL")?;

    let store = TunnelStore::connect(&database_url)
        .await
        .context("connecting to mobile tunnel database")?;

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

async fn health(State(state): State<RouterState>) -> Response {
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
    let target = match resolve_target_for_request(&state, &tunnel_id).await {
        Ok(target) => target,
        Err(response) => return response,
    };

    let upstream_url = build_upstream_url(
        &target.relay_internal_base_url,
        &tunnel_id,
        &path,
        query.as_deref(),
    );
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
    let body = resp.bytes().await.unwrap_or_default();
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
    let target = match state.store.resolve_tunnel_target(&tunnel_id).await {
        Ok(TunnelResolveResult::Resolved(target)) => target,
        Ok(TunnelResolveResult::UnknownTunnel | TunnelResolveResult::TunnelDisabled) => {
            return Ok(())
        }
        Ok(TunnelResolveResult::RelayUnavailable) => {
            warn!(tunnel_id, "relay unavailable for websocket tunnel");
            return Ok(());
        }
        Err(err) => return Err(anyhow::anyhow!("failed to resolve tunnel target: {err}")),
    };
    state
        .store
        .mark_tunnel_accessed(&tunnel_id)
        .await
        .context("marking tunnel accessed")?;

    let upstream = build_upstream_url(
        &target.relay_internal_base_url,
        &tunnel_id,
        &path,
        query.as_deref(),
    );
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

async fn resolve_target_for_request(
    state: &RouterState,
    tunnel_id: &str,
) -> Result<ResolvedTunnelTarget, Response> {
    match state.store.resolve_tunnel_target(tunnel_id).await {
        Ok(TunnelResolveResult::Resolved(target)) => {
            if let Err(err) = state.store.mark_tunnel_accessed(tunnel_id).await {
                warn!("failed to mark tunnel accessed: {err}");
                return Err(StatusCode::SERVICE_UNAVAILABLE.into_response());
            }
            Ok(target)
        }
        Ok(TunnelResolveResult::UnknownTunnel | TunnelResolveResult::TunnelDisabled) => {
            Err(StatusCode::NOT_FOUND.into_response())
        }
        Ok(TunnelResolveResult::RelayUnavailable) => {
            warn!(tunnel_id, "relay unavailable for tunnel");
            Err(StatusCode::SERVICE_UNAVAILABLE.into_response())
        }
        Err(err) => {
            warn!("failed to resolve tunnel target: {err}");
            Err(StatusCode::SERVICE_UNAVAILABLE.into_response())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_upstream_url_preserves_tunnel_and_query() {
        assert_eq!(
            build_upstream_url(
                "https://relay.example/",
                "tunnel-1",
                "/api/health",
                Some("a=1&b=2"),
            ),
            "https://relay.example/t/tunnel-1/api/health?a=1&b=2"
        );
        assert_eq!(
            build_upstream_url("https://relay.example", "tunnel-1", "nested/path", None),
            "https://relay.example/t/tunnel-1/nested/path"
        );
    }

    #[test]
    fn to_ws_url_converts_http_schemes_only() {
        assert_eq!(
            to_ws_url("http://relay.example").unwrap().as_str(),
            "ws://relay.example/"
        );
        assert_eq!(
            to_ws_url("https://relay.example/path").unwrap().as_str(),
            "wss://relay.example/path"
        );
        assert_eq!(
            to_ws_url("wss://relay.example/path").unwrap().as_str(),
            "wss://relay.example/path"
        );
    }

    #[test]
    fn websocket_upgrade_detection_and_header_forwarding_are_narrow() {
        let mut headers = HeaderMap::new();
        headers.insert(axum::http::header::UPGRADE, "websocket".parse().unwrap());
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer secret".parse().unwrap(),
        );
        headers.insert("x-custom", "value".parse().unwrap());

        assert!(is_websocket_upgrade(&headers));
        let forwarded = extract_ws_forward_headers(&headers);
        assert_eq!(forwarded.len(), 1);
        assert_eq!(forwarded[0].0.as_str(), "authorization");
        assert_eq!(forwarded[0].1.to_str().unwrap(), "Bearer secret");

        headers.insert(axum::http::header::UPGRADE, "h2c".parse().unwrap());
        assert!(!is_websocket_upgrade(&headers));
    }
}
