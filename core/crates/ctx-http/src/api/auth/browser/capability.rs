use axum::body::Body;
use axum::http::{Method, Request};
use sha2::Digest;

use super::{derive_browser_query_secret, query_expires_at_within_window, query_param};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BrowserCapabilityAuthScope {
    Blob {
        blob_id: String,
    },
    SessionArtifact {
        session_id: String,
        artifact_id: String,
    },
}

impl BrowserCapabilityAuthScope {
    fn serialize(&self) -> String {
        match self {
            Self::Blob { blob_id } => format!("blob:{blob_id}"),
            Self::SessionArtifact {
                session_id,
                artifact_id,
            } => format!("session_artifact:{session_id}:{artifact_id}"),
        }
    }
}

pub(crate) fn derive_browser_capability_token(
    auth_token: &str,
    scope: &BrowserCapabilityAuthScope,
    expires_at: i64,
) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(b"ctx-browser-capability|");
    hasher.update(scope.serialize().as_bytes());
    hasher.update(b"|");
    hasher.update(expires_at.to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(auth_token.as_bytes());
    hex::encode(hasher.finalize())
}

pub(in crate::api::auth) fn browser_capability_query_token_is_valid(
    req: &Request<Body>,
    auth_token: &str,
) -> bool {
    if req.method() != Method::GET && req.method() != Method::HEAD {
        return false;
    }
    let Some(scope) = browser_capability_scope(req) else {
        return false;
    };
    let Some(query_token) = query_param(req, "token") else {
        return false;
    };
    let Some(expires_at) = query_expires_at_within_window(
        req,
        BROWSER_CAPABILITY_TOKEN_TTL_SECS,
        0,
        BROWSER_CAPABILITY_TOKEN_MAX_FUTURE_SKEW_SECS,
    ) else {
        return false;
    };
    query_token == derive_browser_capability_token(auth_token, &scope, expires_at)
        || query_token
            == derive_browser_capability_token(
                &derive_browser_query_secret(auth_token),
                &scope,
                expires_at,
            )
}

const BROWSER_CAPABILITY_TOKEN_TTL_SECS: i64 = 60 * 60;
const BROWSER_CAPABILITY_TOKEN_MAX_FUTURE_SKEW_SECS: i64 = 60;

fn browser_capability_scope(req: &Request<Body>) -> Option<BrowserCapabilityAuthScope> {
    let path = req.uri().path();
    if let Some(blob_id) = path.strip_prefix("/api/blobs/") {
        let blob_id = blob_id.trim();
        if blob_id.is_empty() || blob_id.contains('/') {
            return None;
        }
        return Some(BrowserCapabilityAuthScope::Blob {
            blob_id: blob_id.to_string(),
        });
    }

    let remainder = path.strip_prefix("/api/sessions/")?;
    let (session_id, suffix) = remainder.split_once('/')?;
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return None;
    }
    let artifact_suffix = suffix.strip_prefix("artifacts/")?;
    let artifact_id = artifact_suffix.trim();
    if artifact_id.is_empty() || artifact_id.contains('/') {
        return None;
    }
    Some(BrowserCapabilityAuthScope::SessionArtifact {
        session_id: session_id.to_string(),
        artifact_id: artifact_id.to_string(),
    })
}
