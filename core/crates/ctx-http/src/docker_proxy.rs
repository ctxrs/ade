use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use hyper::client::conn::http1;
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;
use url::Url;

use ctx_core::ids::SessionId;

use crate::daemon::AppState;

const DOCKER_PROXY_AUTH_REALM: &str = "ctx-docker";

pub fn docker_proxy_enabled() -> bool {
    env_flag("CTX_DOCKER_PROXY_ENABLED")
}

fn env_flag(key: &str) -> bool {
    match std::env::var(key) {
        Ok(value) => value == "1" || value.eq_ignore_ascii_case("true"),
        Err(_) => false,
    }
}

pub async fn docker_host_for_session(state: &AppState, session_id: SessionId) -> Option<String> {
    if !docker_proxy_enabled() {
        return None;
    }
    let token = state.docker_proxy_token_for_session(session_id).await;
    docker_host_for_daemon(&state.daemon_url, &token)
}

pub fn docker_host_for_daemon(base_url: &str, token: &str) -> Option<String> {
    let parsed = Url::parse(base_url).ok()?;
    let host = parsed.host_str()?;
    let port = parsed.port_or_known_default()?;
    Some(format!("tcp://ctx:{token}@{host}:{port}"))
}

pub fn docker_passthrough_host() -> Option<String> {
    if !docker_proxy_enabled() {
        return None;
    }
    let socket = parse_unix_socket_env("CTX_DOCKER_PROXY_SOCKET")?;
    Some(format!("unix://{}", socket.to_string_lossy()))
}

pub fn is_docker_request(req: &Request<Body>) -> bool {
    let path = req.uri().path();
    if path.starts_with("/api/") || path.starts_with("/sessions/") || path.starts_with("/assets/") {
        return false;
    }
    if path == "/" {
        return false;
    }
    if is_versioned_path(path) {
        return true;
    }
    matches!(path, "/_ping" | "/version" | "/info" | "/events")
        || path.starts_with("/containers")
        || path.starts_with("/images")
        || path.starts_with("/networks")
        || path.starts_with("/volumes")
        || path.starts_with("/build")
        || path.starts_with("/exec")
        || path.starts_with("/auth")
        || path.starts_with("/plugins")
        || path.starts_with("/system")
        || path.starts_with("/swarm")
}

fn is_versioned_path(path: &str) -> bool {
    let Some(rest) = path.strip_prefix("/v") else {
        return false;
    };
    rest.chars()
        .next()
        .map(|c| c.is_ascii_digit())
        .unwrap_or(false)
}

pub async fn handle_docker_proxy_request(state: Arc<AppState>, req: Request<Body>) -> Response {
    if !is_docker_request(&req) {
        return StatusCode::NOT_FOUND.into_response();
    }

    let token = match docker_token(req.headers()) {
        Some(token) => token,
        None => return docker_auth_required(),
    };

    let session_id = match state.docker_proxy_session_for_token(&token).await {
        Some(id) => id,
        None => return docker_auth_required(),
    };
    let _session_id = session_id;

    match proxy_docker_request(req).await {
        Ok(resp) => resp,
        Err(status) => status.into_response(),
    }
}

fn docker_token(headers: &HeaderMap) -> Option<String> {
    let header_value = headers.get(header::AUTHORIZATION)?;
    let value = header_value.to_str().ok()?.trim();
    if let Some(token) = value.strip_prefix("Bearer ") {
        let trimmed = token.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    if let Some(encoded) = value.strip_prefix("Basic ") {
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded.as_bytes())
            .ok()?;
        let creds = String::from_utf8(decoded).ok()?;
        let (user, pass) = creds.split_once(':').unwrap_or((creds.as_str(), ""));
        let user = user.trim();
        let pass = pass.trim();
        if user.eq_ignore_ascii_case("ctx") && !pass.is_empty() {
            return Some(pass.to_string());
        }
        if !user.is_empty() {
            return Some(user.to_string());
        }
        if !pass.is_empty() {
            return Some(pass.to_string());
        }
    }
    None
}

fn docker_auth_required() -> Response {
    let mut resp = StatusCode::UNAUTHORIZED.into_response();
    resp.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        header::HeaderValue::from_str(&format!("Basic realm=\"{DOCKER_PROXY_AUTH_REALM}\""))
            .unwrap_or_else(|_| header::HeaderValue::from_static("Basic")),
    );
    resp
}

async fn proxy_docker_request(mut req: Request<Body>) -> Result<Response, StatusCode> {
    strip_auth_headers(&mut req);
    let uri = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");
    *req.uri_mut() = uri.parse().map_err(|_| StatusCode::BAD_REQUEST)?;

    let socket = resolve_upstream_socket();
    let stream = UnixStream::connect(&socket)
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    let io = TokioIo::new(stream);
    let (mut sender, conn) = http1::handshake(io)
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    tokio::spawn(async move {
        if let Err(err) = conn.await {
            tracing::warn!("docker proxy upstream error: {err:#}");
        }
    });

    let resp = sender
        .send_request(req)
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    let (parts, body) = resp.into_parts();
    Ok(Response::from_parts(parts, Body::new(body)))
}

fn strip_auth_headers(req: &mut Request<Body>) {
    req.headers_mut().remove(header::AUTHORIZATION);
    req.headers_mut().remove(header::PROXY_AUTHORIZATION);
}

fn resolve_upstream_socket() -> PathBuf {
    if let Some(path) = parse_unix_socket_env("CTX_DOCKER_PROXY_UPSTREAM") {
        return path;
    }
    if let Some(path) = parse_unix_socket_env("DOCKER_HOST") {
        return path;
    }
    PathBuf::from("/var/run/docker.sock")
}

fn parse_unix_socket_env(key: &str) -> Option<PathBuf> {
    let value = std::env::var(key).ok()?;
    parse_unix_socket_value(&value)
}

fn parse_unix_socket_value(value: &str) -> Option<PathBuf> {
    if let Some(path) = value.strip_prefix("unix://") {
        if !path.is_empty() {
            return Some(PathBuf::from(path));
        }
    }
    if value.starts_with('/') {
        return Some(PathBuf::from(value));
    }
    None
}
