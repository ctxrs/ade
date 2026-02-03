use std::collections::HashMap;
use std::convert::Infallible;
use std::net::{SocketAddr, TcpListener as StdTcpListener};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use base64::Engine;
use bytes::Bytes;
use futures::StreamExt;
use http::header::{HOST, PROXY_AUTHENTICATE, PROXY_AUTHORIZATION};
use http::{Method, Request, Response, StatusCode, Uri};
use http_body::Frame;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::io::copy_bidirectional;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use url::Url;

use ctx_core::ids::WorkspaceId;

use crate::ops_events::OpsEvent;
use crate::ops_events::OpsEvents;
use crate::settings::{ContainerNetworkMode, NetworkContext, NetworkProfile};

const DEFAULT_BIND: &str = "127.0.0.1:0";
const DEFAULT_TOKEN_TTL: Duration = Duration::from_secs(60 * 60);

const LLM_ALLOWLIST: &[&str] = &[
    "api.anthropic.com",
    "chatgpt.com",
    "api.openai.com",
    "api.mistral.ai",
    "api.groq.com",
    "api.cohere.ai",
    "api.together.xyz",
    "api.openrouter.ai",
    "generativelanguage.googleapis.com",
    "vertex.googleapis.com",
    "dashscope.aliyuncs.com",
    "api.deepseek.com",
];

#[derive(Debug, Clone)]
pub struct ProxyPermit {
    pub workspace_id: WorkspaceId,
    pub network_mode: ContainerNetworkMode,
    pub allowlist: Vec<String>,
    pub context: NetworkContext,
    pub expires_at: Option<Instant>,
}

#[derive(Clone)]
struct ProxyState {
    client: reqwest::Client,
    tokens: Arc<Mutex<HashMap<String, ProxyPermit>>>,
    ops_events: OpsEvents,
}

#[derive(Clone)]
pub struct EgressProxy {
    addr: SocketAddr,
    tokens: Arc<Mutex<HashMap<String, ProxyPermit>>>,
}

impl EgressProxy {
    pub fn spawn(ops_events: OpsEvents) -> Result<Self> {
        let listener = StdTcpListener::bind(DEFAULT_BIND).context("binding egress proxy")?;
        listener
            .set_nonblocking(true)
            .context("setting egress proxy nonblocking")?;
        let addr = listener.local_addr().context("reading proxy addr")?;
        let listener =
            TcpListener::from_std(listener).context("converting egress proxy listener")?;
        let tokens = Arc::new(Mutex::new(HashMap::new()));
        let state = Arc::new(ProxyState {
            client: reqwest::Client::new(),
            tokens: tokens.clone(),
            ops_events: ops_events.clone(),
        });
        tokio::spawn(async move {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(value) => value,
                    Err(err) => {
                        tracing::warn!("egress proxy accept failed: {err}");
                        continue;
                    }
                };
                let state = state.clone();
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let service = service_fn(move |req| handle(req, state.clone()));
                    if let Err(err) = http1::Builder::new()
                        .preserve_header_case(true)
                        .title_case_headers(true)
                        .serve_connection(io, service)
                        .with_upgrades()
                        .await
                    {
                        tracing::warn!("egress proxy connection failed: {err}");
                    }
                });
            }
        });
        Ok(Self { addr, tokens })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub async fn register_token(&self, token: String, permit: ProxyPermit) {
        let mut tokens = self.tokens.lock().await;
        tokens.insert(token, permit);
    }

    pub async fn remove_token(&self, token: &str) {
        let mut tokens = self.tokens.lock().await;
        tokens.remove(token);
    }

    pub async fn issue_context_token(
        &self,
        workspace_id: WorkspaceId,
        context: NetworkContext,
        profile: NetworkProfile,
    ) -> String {
        let token = uuid::Uuid::new_v4().to_string();
        let permit = ProxyPermit {
            workspace_id,
            network_mode: profile.mode,
            allowlist: profile.allowlist,
            context,
            expires_at: Some(Instant::now() + DEFAULT_TOKEN_TTL),
        };
        self.register_token(token.clone(), permit).await;
        token
    }

    pub async fn proxy_env_for_context(
        &self,
        workspace_id: WorkspaceId,
        context: NetworkContext,
        profile: NetworkProfile,
        target_host: &str,
    ) -> HashMap<String, String> {
        if matches!(profile.mode, ContainerNetworkMode::All) {
            return HashMap::new();
        }
        let token = self
            .issue_context_token(workspace_id, context.clone(), profile)
            .await;
        let proxy_url = format!(
            "http://ctx-proxy:{}@{}:{}",
            token,
            target_host,
            self.addr.port()
        );
        let mut no_proxy = vec!["localhost", "127.0.0.1", "::1"];
        if !no_proxy.contains(&target_host) {
            no_proxy.push(target_host);
        }
        if target_host != "host.containers.internal" {
            no_proxy.push("host.containers.internal");
        }
        let mut env = HashMap::new();
        env.insert("HTTP_PROXY".to_string(), proxy_url.clone());
        env.insert("HTTPS_PROXY".to_string(), proxy_url.clone());
        env.insert("ALL_PROXY".to_string(), proxy_url);
        env.insert("NO_PROXY".to_string(), no_proxy.join(","));
        env.insert(
            "CTX_NETWORK_CONTEXT".to_string(),
            context.as_str().to_string(),
        );
        env
    }
}

fn emit_denied(
    ops_events: &OpsEvents,
    permit: Option<&ProxyPermit>,
    host: &str,
    port: u16,
    reason: &str,
) {
    let mut event = OpsEvent::new("warn", "egress_denied");
    if let Some(permit) = permit {
        event.meta = Some(serde_json::json!({
            "workspace_id": permit.workspace_id.0,
            "host": host,
            "port": port,
            "mode": permit.network_mode,
            "context": permit.context,
            "reason": reason,
        }));
    } else {
        event.meta = Some(serde_json::json!({
            "host": host,
            "port": port,
            "reason": reason,
        }));
    }
    ops_events.emit(event);
}

async fn handle(
    req: Request<Incoming>,
    state: Arc<ProxyState>,
) -> Result<Response<BoxBody<Bytes, Infallible>>, Infallible> {
    let (host, port) = match extract_host_port(&req) {
        Some(value) => value,
        None => {
            return Ok(response_with_status(
                StatusCode::BAD_REQUEST,
                "missing host",
            ))
        }
    };

    let token = extract_proxy_token(req.headers());
    if token.is_none() {
        emit_denied(&state.ops_events, None, &host, port, "missing_token");
        return Ok(response_proxy_auth());
    }
    let mut permit = if let Some(token) = token.as_deref() {
        let tokens = state.tokens.lock().await;
        tokens.get(token).cloned()
    } else {
        None
    };

    if let (Some(token), Some(permit_ref)) = (token.as_deref(), permit.as_ref()) {
        if is_permit_expired(permit_ref, Instant::now()) {
            let mut tokens = state.tokens.lock().await;
            tokens.remove(token);
            emit_denied(&state.ops_events, Some(permit_ref), &host, port, "expired");
            permit = None;
        }
    }

    if !allowed(&host, permit.as_ref()) {
        emit_denied(&state.ops_events, permit.as_ref(), &host, port, "blocked");
        return Ok(response_with_status(
            StatusCode::FORBIDDEN,
            "egress blocked",
        ));
    }
    if let Some(permit) = permit.as_ref() {
        tracing::debug!(
            workspace_id = %permit.workspace_id.0,
            context = permit.context.as_str(),
            host = %host,
            port = port,
            "egress proxy request"
        );
    }

    if req.method() == Method::CONNECT {
        let target = format!("{host}:{port}");
        tokio::spawn(async move {
            match hyper::upgrade::on(req).await {
                Ok(upgraded) => match TcpStream::connect(&target).await {
                    Ok(mut server) => {
                        let mut upgraded = TokioIo::new(upgraded);
                        let _ = copy_bidirectional(&mut upgraded, &mut server).await;
                    }
                    Err(err) => {
                        tracing::warn!("proxy connect failed: {err}");
                    }
                },
                Err(err) => {
                    tracing::warn!("proxy upgrade failed: {err}");
                }
            }
        });
        return Ok(response_empty(StatusCode::OK));
    }

    let (mut parts, body) = req.into_parts();
    let uri = if parts.uri.scheme().is_none() {
        match rebuild_proxy_uri(&parts.uri, &host, port) {
            Some(uri) => uri,
            None => {
                return Ok(response_with_status(
                    StatusCode::BAD_REQUEST,
                    "missing proxy uri",
                ))
            }
        }
    } else {
        parts.uri.clone()
    };

    parts.headers.remove(PROXY_AUTHORIZATION);
    parts.headers.remove(HOST);

    let body_bytes = match body.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(err) => {
            return Ok(response_with_status(
                StatusCode::BAD_REQUEST,
                format!("proxy error: {err}"),
            ))
        }
    };

    let mut outbound = state
        .client
        .request(parts.method.clone(), uri.to_string())
        .headers(parts.headers);
    outbound = outbound.body(body_bytes);

    let resp = match outbound.send().await {
        Ok(resp) => resp,
        Err(err) => {
            return Ok(response_with_status(
                StatusCode::BAD_GATEWAY,
                format!("proxy error: {err}"),
            ))
        }
    };

    let mut builder = Response::builder().status(resp.status());
    for (key, value) in resp.headers().iter() {
        builder = builder.header(key, value);
    }
    let stream = resp.bytes_stream().filter_map(|item| async {
        match item {
            Ok(bytes) => Some(Ok(Frame::data(bytes))),
            Err(err) => {
                tracing::warn!("proxy stream error: {err}");
                None
            }
        }
    });
    let body = BoxBody::new(StreamBody::new(stream));
    Ok(builder
        .body(body)
        .unwrap_or_else(|_| response_with_status(StatusCode::BAD_GATEWAY, "proxy error")))
}

fn response_with_status(
    status: StatusCode,
    body: impl Into<Bytes>,
) -> Response<BoxBody<Bytes, Infallible>> {
    Response::builder()
        .status(status)
        .body(BoxBody::new(Full::new(body.into())))
        .unwrap_or_else(|_| {
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(BoxBody::new(Full::new(Bytes::from("proxy error"))))
                .unwrap()
        })
}

fn response_empty(status: StatusCode) -> Response<BoxBody<Bytes, Infallible>> {
    Response::builder()
        .status(status)
        .body(BoxBody::new(Full::new(Bytes::new())))
        .unwrap_or_else(|_| {
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(BoxBody::new(Full::new(Bytes::from("proxy error"))))
                .unwrap()
        })
}

fn response_proxy_auth() -> Response<BoxBody<Bytes, Infallible>> {
    Response::builder()
        .status(StatusCode::PROXY_AUTHENTICATION_REQUIRED)
        .header(PROXY_AUTHENTICATE, "Basic realm=\"ctx-proxy\"")
        .body(BoxBody::new(Full::new(Bytes::from("proxy auth required"))))
        .unwrap_or_else(|_| {
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(BoxBody::new(Full::new(Bytes::from("proxy error"))))
                .unwrap()
        })
}

fn extract_proxy_token(headers: &http::HeaderMap) -> Option<String> {
    let value = headers.get(PROXY_AUTHORIZATION)?;
    let raw = value.to_str().ok()?.trim();
    if let Some(encoded) = raw.strip_prefix("Basic ") {
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded.as_bytes())
            .ok()?;
        let decoded = String::from_utf8(decoded).ok()?;
        let mut parts = decoded.splitn(2, ':');
        let _user = parts.next()?;
        let token = parts.next().unwrap_or_default();
        if token.trim().is_empty() {
            None
        } else {
            Some(token.to_string())
        }
    } else {
        None
    }
}

fn extract_host_port(req: &Request<Incoming>) -> Option<(String, u16)> {
    if req.method() == Method::CONNECT {
        let authority = req.uri().authority()?.as_str();
        let mut parts = authority.splitn(2, ':');
        let host = parts.next()?.to_string();
        let port = parts
            .next()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(443);
        return Some((host, port));
    }

    if let Some(authority) = req.uri().authority() {
        let mut host = authority.host().to_string();
        let port = authority.port_u16().unwrap_or(80);
        if host.is_empty() {
            host = authority.as_str().to_string();
        }
        return Some((host, port));
    }

    let host_header = req.headers().get(HOST)?;
    let host_header = host_header.to_str().ok()?;
    let mut parts = host_header.splitn(2, ':');
    let host = parts.next()?.to_string();
    let port = parts
        .next()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(80);
    Some((host, port))
}

fn rebuild_proxy_uri(uri: &Uri, host: &str, port: u16) -> Option<Uri> {
    let path = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");
    let scheme = uri.scheme_str().unwrap_or("http");
    let host = if port == 80 {
        host.to_string()
    } else {
        format!("{host}:{port}")
    };
    let uri = format!("{scheme}://{host}{path}");
    uri.parse().ok()
}

fn allowed(host: &str, permit: Option<&ProxyPermit>) -> bool {
    let Some(permit) = permit else {
        return false;
    };
    if matches!(permit.network_mode, ContainerNetworkMode::All) {
        return true;
    }
    let host = host.to_ascii_lowercase();
    let mut allowlist = Vec::new();
    if matches!(permit.network_mode, ContainerNetworkMode::LlmOnly) {
        allowlist.extend(
            LLM_ALLOWLIST
                .iter()
                .filter_map(|entry| normalize_allowlist_entry(entry)),
        );
    }
    if matches!(permit.network_mode, ContainerNetworkMode::Allowlist) {
        allowlist.extend(
            permit
                .allowlist
                .iter()
                .filter_map(|entry| normalize_allowlist_entry(entry)),
        );
    }
    allowlist.iter().any(|entry| host_matches(&host, entry))
}

fn normalize_allowlist_entry(entry: &str) -> Option<String> {
    let trimmed = entry.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(url) = Url::parse(trimmed) {
        if let Some(host) = url.host_str() {
            return Some(host.to_ascii_lowercase());
        }
    }
    let host = trimmed
        .split('/')
        .next()
        .unwrap_or(trimmed)
        .split(':')
        .next()
        .unwrap_or(trimmed)
        .trim();
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

fn host_matches(host: &str, entry: &str) -> bool {
    host == entry || host.ends_with(&format!(".{entry}"))
}

fn is_permit_expired(permit: &ProxyPermit, now: Instant) -> bool {
    permit
        .expires_at
        .is_some_and(|expires_at| now >= expires_at)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_matches_allows_suffix() {
        assert!(host_matches("api.openai.com", "openai.com"));
        assert!(host_matches("openai.com", "openai.com"));
        assert!(!host_matches("evilopenai.com", "openai.com"));
    }

    #[test]
    fn allowlist_enforces_llm_only() {
        let permit = ProxyPermit {
            workspace_id: WorkspaceId(uuid::Uuid::new_v4()),
            network_mode: ContainerNetworkMode::LlmOnly,
            allowlist: Vec::new(),
            context: NetworkContext::AgentDefault,
            expires_at: None,
        };
        assert!(allowed("api.openai.com", Some(&permit)));
        assert!(!allowed("example.com", Some(&permit)));
    }

    #[test]
    fn allowlist_enforces_custom() {
        let permit = ProxyPermit {
            workspace_id: WorkspaceId(uuid::Uuid::new_v4()),
            network_mode: ContainerNetworkMode::Allowlist,
            allowlist: vec!["example.com".to_string()],
            context: NetworkContext::AgentDefault,
            expires_at: None,
        };
        assert!(allowed("api.example.com", Some(&permit)));
        assert!(!allowed("api.openai.com", Some(&permit)));
    }

    #[test]
    fn permit_expiration_checks_deadline() {
        let permit = ProxyPermit {
            workspace_id: WorkspaceId(uuid::Uuid::new_v4()),
            network_mode: ContainerNetworkMode::LlmOnly,
            allowlist: Vec::new(),
            context: NetworkContext::AgentDefault,
            expires_at: Some(Instant::now() - Duration::from_secs(1)),
        };
        assert!(is_permit_expired(&permit, Instant::now()));
    }

    #[tokio::test]
    async fn proxy_env_sets_context() {
        let tmp = tempfile::tempdir().unwrap();
        let proxy = EgressProxy::spawn(OpsEvents::new(tmp.path().to_path_buf())).unwrap();
        let profile = NetworkProfile {
            mode: ContainerNetworkMode::Allowlist,
            allowlist: vec!["example.com".to_string()],
        };
        let env = proxy
            .proxy_env_for_context(
                WorkspaceId(uuid::Uuid::new_v4()),
                NetworkContext::MergeQueue,
                profile,
                "127.0.0.1",
            )
            .await;
        assert_eq!(
            env.get("CTX_NETWORK_CONTEXT").map(String::as_str),
            Some("merge_queue")
        );
        assert!(env
            .get("HTTP_PROXY")
            .map(|v| v.contains("ctx-proxy:"))
            .unwrap_or(false));
    }

    #[tokio::test]
    async fn proxy_env_skips_all_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let proxy = EgressProxy::spawn(OpsEvents::new(tmp.path().to_path_buf())).unwrap();
        let profile = NetworkProfile {
            mode: ContainerNetworkMode::All,
            allowlist: Vec::new(),
        };
        let env = proxy
            .proxy_env_for_context(
                WorkspaceId(uuid::Uuid::new_v4()),
                NetworkContext::UserShell,
                profile,
                "127.0.0.1",
            )
            .await;
        assert!(!env.contains_key("HTTP_PROXY"));
        assert!(!env.contains_key("CTX_NETWORK_CONTEXT"));
    }
}
