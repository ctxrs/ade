use axum::body::Body;
use axum::http::Request;
use sha2::Digest;
use url::form_urlencoded;

mod capability;
mod mcp;
mod stream;

pub(super) use capability::browser_capability_query_token_is_valid;
#[cfg(test)]
pub(crate) use capability::{derive_browser_capability_token, BrowserCapabilityAuthScope};
pub(super) use mcp::{scoped_mcp_route, ScopedMcpRoute};
pub(super) use stream::browser_stream_query_token_is_valid;
#[cfg(test)]
pub(crate) use stream::{derive_browser_stream_token, BrowserStreamAuthScope};

pub(super) fn is_websocket_upgrade(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get(axum::http::header::UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
        || headers.contains_key("sec-websocket-key")
}

pub(super) fn query_param(req: &Request<Body>, key: &str) -> Option<String> {
    let query = req.uri().query()?;
    form_urlencoded::parse(query.as_bytes())
        .find(|(candidate, _)| candidate == key)
        .map(|(_, value)| value.into_owned())
}

pub(crate) fn derive_browser_query_secret(auth_token: &str) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(b"ctx-desktop-browser-query-secret|");
    hasher.update(auth_token.as_bytes());
    hex::encode(hasher.finalize())
}

fn browser_query_secret_bearer_route_allowed(req: &Request<Body>) -> bool {
    req.uri().path().starts_with("/api/")
}

pub(super) fn browser_query_secret_bearer_is_valid(req: &Request<Body>, auth_token: &str) -> bool {
    browser_query_secret_bearer_route_allowed(req)
        && req
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .is_some_and(|value| value == derive_browser_query_secret(auth_token))
}

pub(super) fn query_expires_at_within_window(
    req: &Request<Body>,
    ttl_secs: i64,
    max_past_skew_secs: i64,
    max_future_skew_secs: i64,
) -> Option<i64> {
    let expires_at = query_param(req, "expires_at").and_then(|value| value.parse().ok())?;
    let now = chrono::Utc::now().timestamp();
    if expires_at < now - max_past_skew_secs {
        return None;
    }
    if expires_at > now + ttl_secs + max_future_skew_secs {
        return None;
    }
    Some(expires_at)
}
