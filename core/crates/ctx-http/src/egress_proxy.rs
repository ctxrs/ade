use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use hyper_util::rt::TokioIo;
use tokio::io::copy_bidirectional;
use tokio::net::TcpStream;
use url::Url;

use ctx_core::ids::SessionId;

use crate::daemon::AppState;
use crate::settings::{NetworkProfile, NetworkSettings};

const PROXY_AUTH_REALM: &str = "ctx";
const PROXY_BODY_LIMIT: usize = 32 * 1024 * 1024;

#[derive(Default)]
pub struct ProxySessionStore {
    by_session: HashMap<SessionId, String>,
    by_token: HashMap<String, SessionId>,
}

impl ProxySessionStore {
    pub fn token_for_session(&mut self, session_id: SessionId) -> String {
        if let Some(token) = self.by_session.get(&session_id) {
            return token.clone();
        }
        let token = uuid::Uuid::new_v4().to_string();
        self.by_session.insert(session_id, token.clone());
        self.by_token.insert(token.clone(), session_id);
        token
    }

    pub fn session_for_token(&self, token: &str) -> Option<SessionId> {
        self.by_token.get(token).copied()
    }
}

pub fn proxy_url_for_daemon(base_url: &str, token: &str) -> Option<String> {
    let parsed = Url::parse(base_url).ok()?;
    let host = parsed.host_str()?;
    let port = parsed.port_or_known_default()?;
    Some(format!("http://ctx:{token}@{host}:{port}"))
}

pub fn build_no_proxy(daemon_url: &str) -> String {
    let mut entries = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
    ];
    if let Ok(url) = Url::parse(daemon_url) {
        if let Some(host) = url.host_str() {
            entries.push(host.to_string());
        }
    }
    entries.sort();
    entries.dedup();
    entries.join(",")
}

pub fn is_proxy_request(req: &Request<Body>) -> bool {
    if req.method() == Method::CONNECT {
        return true;
    }
    if req.uri().scheme().is_some() || req.uri().authority().is_some() {
        return true;
    }
    req.headers().contains_key(header::PROXY_AUTHORIZATION)
}

pub async fn handle_proxy_request(state: Arc<AppState>, req: Request<Body>) -> Response {
    if !is_proxy_request(&req) {
        return StatusCode::NOT_FOUND.into_response();
    }

    let token = match proxy_token(req.headers()) {
        Some(token) => token,
        None => return proxy_auth_required(),
    };

    let session_id = match state.proxy_session_for_token(&token).await {
        Some(id) => id,
        None => return proxy_auth_required(),
    };

    let settings = crate::settings::load_settings(&state.data_root).await;
    let network = settings.network.unwrap_or_default();

    if req.method() == Method::CONNECT {
        return handle_connect(&state, session_id, network, req).await;
    }

    handle_http(&state, session_id, network, req).await
}

fn proxy_token(headers: &HeaderMap) -> Option<String> {
    let header_value = headers.get(header::PROXY_AUTHORIZATION)?;
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

fn proxy_auth_required() -> Response {
    let mut resp = StatusCode::PROXY_AUTHENTICATION_REQUIRED.into_response();
    resp.headers_mut().insert(
        header::PROXY_AUTHENTICATE,
        header::HeaderValue::from_str(&format!("Basic realm=\"{PROXY_AUTH_REALM}\""))
            .unwrap_or_else(|_| header::HeaderValue::from_static("Basic")),
    );
    resp
}

#[derive(Debug)]
struct ProxyTarget {
    url: Url,
    host: String,
    port: u16,
}

fn resolve_http_target(req: &Request<Body>) -> Result<ProxyTarget, StatusCode> {
    let url = if req.uri().scheme().is_some() {
        Url::parse(&req.uri().to_string()).map_err(|_| StatusCode::BAD_REQUEST)?
    } else {
        let host = req
            .headers()
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .ok_or(StatusCode::BAD_REQUEST)?;
        let base = format!("http://{host}/");
        let base = Url::parse(&base).map_err(|_| StatusCode::BAD_REQUEST)?;
        let path = req
            .uri()
            .path_and_query()
            .map(|pq| pq.as_str())
            .unwrap_or("/");
        base.join(path).map_err(|_| StatusCode::BAD_REQUEST)?
    };
    let host = url.host_str().ok_or(StatusCode::BAD_REQUEST)?.to_string();
    let port = url.port_or_known_default().ok_or(StatusCode::BAD_REQUEST)?;
    Ok(ProxyTarget { url, host, port })
}

fn profile_allows(profile: NetworkProfile, host: &str, port: u16) -> bool {
    match profile {
        NetworkProfile::Full => true,
        NetworkProfile::None | NetworkProfile::McpOnly => false,
        NetworkProfile::DepsOnly | NetworkProfile::DepsPlusMcp => {
            let host = host.to_ascii_lowercase();
            if is_dependency_host(&host) {
                if port == 22 || port == 9418 {
                    return is_git_host(&host);
                }
                return matches!(port, 80 | 443);
            }
            false
        }
    }
}

fn is_dependency_host(host: &str) -> bool {
    const HOSTS: &[&str] = &[
        "registry.npmjs.org",
        "registry.yarnpkg.com",
        "pypi.org",
        "files.pythonhosted.org",
        "test.pypi.org",
        "crates.io",
        "index.crates.io",
        "static.crates.io",
        "rubygems.org",
        "api.rubygems.org",
        "repo.maven.apache.org",
        "repo1.maven.org",
        "repo.maven.org",
        "central.maven.org",
        "packagist.org",
        "proxy.golang.org",
        "sum.golang.org",
        "github.com",
        "raw.githubusercontent.com",
        "objects.githubusercontent.com",
        "codeload.github.com",
        "gitlab.com",
        "bitbucket.org",
        "ssh.github.com",
    ];
    HOSTS.iter().any(|allowed| host_matches(host, allowed))
}

fn is_git_host(host: &str) -> bool {
    const HOSTS: &[&str] = &[
        "github.com",
        "gitlab.com",
        "bitbucket.org",
        "ssh.github.com",
    ];
    HOSTS.iter().any(|allowed| host_matches(host, allowed))
}

fn host_matches(host: &str, allowed: &str) -> bool {
    if host == allowed {
        return true;
    }
    host.ends_with(&format!(".{allowed}"))
}

fn strip_proxy_headers(headers: &HeaderMap) -> HeaderMap {
    let mut out = HeaderMap::new();
    for (name, value) in headers.iter() {
        if is_hop_header(name) {
            continue;
        }
        out.insert(name.clone(), value.clone());
    }
    out
}

fn is_hop_header(name: &header::HeaderName) -> bool {
    matches!(
        name,
        &header::CONNECTION
            | &header::PROXY_AUTHORIZATION
            | &header::TE
            | &header::TRAILER
            | &header::TRANSFER_ENCODING
            | &header::UPGRADE
    ) || matches!(name.as_str(), "proxy-connection" | "keep-alive")
}

async fn handle_http(
    state: &Arc<AppState>,
    _session_id: SessionId,
    network: NetworkSettings,
    req: Request<Body>,
) -> Response {
    let target = match resolve_http_target(&req) {
        Ok(target) => target,
        Err(status) => return status.into_response(),
    };

    if !profile_allows(network.profile, &target.host, target.port) {
        return StatusCode::FORBIDDEN.into_response();
    }

    let method = req.method().clone();
    let headers = strip_proxy_headers(req.headers());
    let body = match axum::body::to_bytes(req.into_body(), PROXY_BODY_LIMIT).await {
        Ok(bytes) => bytes,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };

    let mut builder = state.proxy_client().request(method, target.url.to_string());
    for (name, value) in headers.iter() {
        builder = builder.header(name, value);
    }
    if !body.is_empty() {
        builder = builder.body(body);
    }

    let resp: reqwest::Response = match builder.send().await {
        Ok(resp) => resp,
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };

    let status = resp.status();
    let mut out = Response::builder().status(status);
    for (name, value) in resp.headers().iter() {
        if is_hop_header(name) {
            continue;
        }
        out = out.header(name, value);
    }
    let bytes = match resp.bytes().await {
        Ok(bytes) => bytes,
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };
    out.body(Body::from(bytes))
        .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
}

async fn handle_connect(
    _state: &Arc<AppState>,
    _session_id: SessionId,
    network: NetworkSettings,
    req: Request<Body>,
) -> Response {
    let authority = req.uri().authority().cloned().or_else(|| {
        req.headers()
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse().ok())
    });
    let Some(authority) = authority else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let host = authority.host().to_string();
    let port = authority.port_u16().unwrap_or(443);

    if !profile_allows(network.profile, &host, port) {
        return StatusCode::FORBIDDEN.into_response();
    }

    let on_upgrade = hyper::upgrade::on(req);
    let upstream = match TcpStream::connect((host.as_str(), port)).await {
        Ok(stream) => stream,
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };

    tokio::spawn(async move {
        if let Ok(upgraded) = on_upgrade.await {
            let mut upgraded = TokioIo::new(upgraded);
            let mut upstream = upstream;
            let _ = copy_bidirectional(&mut upgraded, &mut upstream).await;
        }
    });

    Response::builder()
        .status(StatusCode::OK)
        .body(Body::empty())
        .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
}

#[cfg(test)]
mod tests {
    use super::{host_matches, profile_allows};
    use crate::settings::NetworkProfile;

    #[test]
    fn matches_dependency_subdomains() {
        assert!(host_matches("foo.github.com", "github.com"));
        assert!(host_matches("github.com", "github.com"));
        assert!(!host_matches("github.co", "github.com"));
    }

    #[test]
    fn deps_only_allows_registry_https() {
        assert!(profile_allows(
            NetworkProfile::DepsOnly,
            "registry.npmjs.org",
            443
        ));
        assert!(!profile_allows(
            NetworkProfile::DepsOnly,
            "example.com",
            443
        ));
    }

    #[test]
    fn deps_only_allows_git_over_ssh() {
        assert!(profile_allows(NetworkProfile::DepsOnly, "github.com", 22));
        assert!(!profile_allows(
            NetworkProfile::DepsOnly,
            "registry.npmjs.org",
            22
        ));
    }
}
