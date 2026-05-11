use axum::body::Body;
use axum::http::{Method, Request};
use scope::browser_stream_scope;
pub(crate) use scope::BrowserStreamAuthScope;
use sha2::Digest;

use super::{derive_browser_query_secret, query_expires_at_within_window, query_param};

#[path = "stream/scope.rs"]
mod scope;

pub(crate) fn derive_browser_stream_token(
    auth_token: &str,
    scope: &BrowserStreamAuthScope,
    expires_at: i64,
) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(b"ctx-browser-stream|");
    hasher.update(scope.serialize().as_bytes());
    hasher.update(b"|");
    hasher.update(expires_at.to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(auth_token.as_bytes());
    hex::encode(hasher.finalize())
}

pub(in crate::api::auth) fn browser_stream_query_token_is_valid(
    req: &Request<Body>,
    auth_token: &str,
) -> bool {
    if req.method() != Method::GET {
        return false;
    }
    let Some(scope) = browser_stream_scope(req) else {
        return false;
    };
    let Some(query_token) = query_param(req, "token") else {
        return false;
    };
    let Some(expires_at) = query_expires_at_within_window(
        req,
        BROWSER_STREAM_TOKEN_TTL_SECS,
        BROWSER_STREAM_TOKEN_MAX_PAST_SKEW_SECS,
        BROWSER_STREAM_TOKEN_MAX_FUTURE_SKEW_SECS,
    ) else {
        return false;
    };
    query_token == derive_browser_stream_token(auth_token, &scope, expires_at)
        || query_token
            == derive_browser_stream_token(
                &derive_browser_query_secret(auth_token),
                &scope,
                expires_at,
            )
}

const BROWSER_STREAM_TOKEN_TTL_SECS: i64 = 5 * 60;
const BROWSER_STREAM_TOKEN_MAX_PAST_SKEW_SECS: i64 = 10 * 60;
const BROWSER_STREAM_TOKEN_MAX_FUTURE_SKEW_SECS: i64 = 10 * 60;
