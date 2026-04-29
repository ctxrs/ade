use std::collections::HashMap;
use std::path::Path as StdPath;
use std::sync::Arc;
use std::time::Instant;

use axum::body::{Body, Bytes};
use axum::extract::{Extension, MatchedPath, Path, Query, State};
use axum::http::header;
use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::Engine;
use opentelemetry::trace::SpanKind;
use opentelemetry::KeyValue;
use serde::{Deserialize, Serialize};
use tower::util::ServiceExt;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use url::Url;

pub(crate) mod artifacts;
mod auth;
mod demo;
pub(crate) mod errors;
mod execution;
mod extractors;
mod merge_queue_api;
mod mobile_access;
mod mobile_scopes;
mod provider_catalog;
mod provider_launch;
pub(crate) mod provider_probe_auth;
pub(crate) mod providers;
mod repo;
mod routes;
pub(crate) mod sessions;
mod settings;
pub(crate) mod shared;
pub(crate) mod tasks;
mod telemetry;
mod terminals;
mod types;
mod updates;
mod web_sessions;
mod workspaces;
mod ws;

#[cfg(test)]
pub(crate) use artifacts::open_canonical_session_artifact_file;
#[cfg(test)]
pub(crate) use auth::{
    derive_browser_capability_token, derive_browser_query_secret, derive_browser_stream_token,
    BrowserCapabilityAuthScope, BrowserStreamAuthScope,
};

use artifacts::*;
use execution::*;
use merge_queue_api::*;
use mobile_access::*;
use mobile_scopes::*;
use providers::*;
use repo::*;
use sessions::*;
use settings::*;
use tasks::*;
use telemetry::*;
use terminals::*;
use types::*;
use updates::*;
use web_sessions::*;
use workspaces::*;

use auth::{
    auth_middleware, generate_mobile_api_token, generate_pairing_token, hash_api_token,
    hash_pairing_token, load_mobile_auth_context_for_profile, MobileAuthContext,
};
use demo::*;
use errors::ApiErrorResp;
use ws::{
    dictation_livekit_stream_ws, mobile_secure_workspace_stream_ws, terminal_stream_ws,
    web_session_signal, workspace_active_snapshot_stream_ws,
};

use ctx_core::{ids::*, models::*};
use ctx_store::store::MobileDeviceUpsert;

use crate::daemon::AppState;
use crate::installer;
use crate::logs;
use crate::merge_queue;
use crate::perf_telemetry::{PerfMetric, PerfMetricKind};
use crate::resource_utilization;
use crate::title_generation_local;
use crate::web_sessions::{
    render_web_session_view, WebSessionInfo, WebSessionRunRequest, WebSessionRunResponse,
    WebSessionViewport,
};
use ctx_provider_install::install_state::InstallId;
use ctx_providers::adapters::ProviderStatus;

pub(super) fn is_sensitive_key(key: &str) -> bool {
    ctx_core::redaction::is_sensitive_key(key)
}

pub(super) fn redact_json_value(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                if is_sensitive_key(&k) {
                    out.insert(k, serde_json::Value::String("[REDACTED]".to_string()));
                    continue;
                }
                out.insert(k, redact_json_value(v));
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.into_iter().map(redact_json_value).collect())
        }
        serde_json::Value::String(s) => serde_json::Value::String(logs::redact_sensitive(&s)),
        other => other,
    }
}

fn header_first_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let value = headers.get(name)?.to_str().ok()?;
    Some(value.split(',').next()?.trim().to_string())
}

fn parse_forwarded_header(value: &str) -> (Option<String>, Option<String>) {
    let mut proto = None;
    let mut host = None;
    let first = value.split(',').next().unwrap_or(value);
    for part in first.split(';') {
        let part = part.trim();
        if let Some(raw) = part.strip_prefix("proto=") {
            let clean = raw.trim().trim_matches('"').trim_matches('\'');
            if !clean.is_empty() {
                proto = Some(clean.to_string());
            }
        } else if let Some(raw) = part.strip_prefix("host=") {
            let clean = raw.trim().trim_matches('"').trim_matches('\'');
            if !clean.is_empty() {
                host = Some(clean.to_string());
            }
        }
    }
    (proto, host)
}

fn resolve_request_base_url(
    headers: &HeaderMap,
    fallback: &str,
    public_base_url: Option<&str>,
) -> Option<String> {
    if let Some(public_base_url) = public_base_url {
        let trimmed = public_base_url.trim().trim_end_matches('/');
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    let fallback = fallback.trim_end_matches('/');
    let fallback_url = Url::parse(fallback).ok();
    let fallback_base = fallback_url.as_ref().and_then(|url| {
        let host = url.host_str()?;
        let host = match url.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        };
        if is_safe_request_base_host(&host) && matches!(url.scheme(), "http" | "https") {
            Some(format!("{}://{}", url.scheme(), host.trim_end_matches('/')))
        } else {
            None
        }
    });

    let (forwarded_proto, forwarded_host) = headers
        .get(header::FORWARDED)
        .and_then(|value| value.to_str().ok())
        .map(parse_forwarded_header)
        .unwrap_or((None, None));

    let proto = forwarded_proto
        .or_else(|| header_first_value(headers, "x-forwarded-proto"))
        .unwrap_or_else(|| {
            fallback_url
                .as_ref()
                .map(|url| url.scheme().to_string())
                .unwrap_or_else(|| "http".to_string())
        })
        .trim()
        .trim_end_matches(':')
        .to_ascii_lowercase();
    let host = forwarded_host
        .or_else(|| header_first_value(headers, "x-forwarded-host"))
        .or_else(|| header_first_value(headers, header::HOST.as_str()));

    match host {
        Some(host)
            if is_safe_request_base_host(&host) && matches!(proto.as_str(), "http" | "https") =>
        {
            Some(format!("{}://{}", proto, host.trim_end_matches('/')))
        }
        Some(_) => None,
        None => fallback_base,
    }
}

fn public_route_url(base_url: &str, route_path: &str) -> Option<String> {
    let mut base = Url::parse(base_url).ok()?;
    let normalized_base_path = match base.path().trim_end_matches('/') {
        "" => "/".to_string(),
        path => format!("{path}/"),
    };
    base.set_path(&normalized_base_path);
    base.join(route_path.trim_start_matches('/'))
        .ok()
        .map(|url| url.to_string())
}

fn public_websocket_url(base_url: &str, route_path: &str) -> Option<String> {
    let mut url =
        public_route_url(base_url, route_path).and_then(|joined| Url::parse(&joined).ok())?;
    match url.scheme() {
        "http" => {
            url.set_scheme("ws").ok()?;
        }
        "https" => {
            url.set_scheme("wss").ok()?;
        }
        _ => return None,
    }
    Some(url.to_string())
}

fn is_loopback_host(host: &str) -> bool {
    let normalized = host.trim().trim_matches('[').trim_matches(']');
    normalized.eq_ignore_ascii_case("localhost")
        || normalized.eq_ignore_ascii_case("tauri.localhost")
        || normalized == "127.0.0.1"
        || normalized == "::1"
}

fn is_safe_request_base_host(host: &str) -> bool {
    let trimmed = host.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return false;
    }
    let parsed = match Url::parse(&format!("http://{trimmed}")) {
        Ok(url) => url,
        Err(_) => return false,
    };
    parsed.host_str().map(is_loopback_host).unwrap_or(false)
}

fn is_allowed_desktop_worker_origin(origin: &HeaderValue) -> bool {
    let Ok(raw) = origin.to_str() else {
        return false;
    };
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("tauri://localhost") {
        return true;
    }
    let Ok(url) = Url::parse(trimmed) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    match url.scheme() {
        "http" | "https" => is_loopback_host(host),
        "tauri" => host.eq_ignore_ascii_case("localhost"),
        _ => false,
    }
}

fn daemon_cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin, _| {
            is_allowed_desktop_worker_origin(origin)
        }))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::HeaderName::from_static("traceparent"),
            header::HeaderName::from_static("x-ctx-run-id"),
        ])
}

pub fn router(state: Arc<AppState>) -> axum::Router {
    let auth_state = state.clone();
    let perf_state = state.clone();
    let api = routes::api_routes()
        .route_layer(middleware::from_fn_with_state(perf_state, perf_middleware))
        .layer(middleware::from_fn_with_state(auth_state, auth_middleware))
        .layer(daemon_cors_layer())
        .with_state(state);

    let dist_dir = std::env::var("CTX_WEB_DIST").unwrap_or_else(|_| "apps/web/dist".into());
    let index_path = format!("{dist_dir}/index.html");
    api.fallback_service(ServeDir::new(dist_dir).not_found_service(ServeFile::new(index_path)))
}

const MOBILE_API_MIN_VERSION: i64 = 1;
const MOBILE_API_MAX_VERSION: i64 = 1;

#[derive(Debug, Serialize)]
struct HealthCompatibility {
    desktop_exact_version: String,
    desktop_build_id: String,
    desktop_dev_instance_id: String,
    protocol_compatibility_token: String,
    mobile_api_min: i64,
    mobile_api_max: i64,
}

#[derive(Debug, Serialize)]
struct HealthResp {
    version: String,
    daemon_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    daemon_url: Option<String>,
    auth_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    open_file_limit: Option<crate::process_limits::OpenFileLimitSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    storage: Option<crate::storage_guard::StorageGuardStatus>,
    compatibility: HealthCompatibility,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_request_base_url_accepts_loopback_host_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("127.0.0.1:4455"));
        assert_eq!(
            resolve_request_base_url(&headers, "http://127.0.0.1:4321", None),
            Some("http://127.0.0.1:4455".to_string())
        );
    }

    #[test]
    fn resolve_request_base_url_rejects_non_loopback_host_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("evil.example"));
        assert_eq!(
            resolve_request_base_url(&headers, "http://127.0.0.1:4321", None),
            None
        );
    }

    #[test]
    fn resolve_request_base_url_rejects_non_http_forwarded_proto() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::FORWARDED,
            HeaderValue::from_static("proto=javascript;host=127.0.0.1:4455"),
        );
        assert_eq!(
            resolve_request_base_url(&headers, "http://127.0.0.1:4321", None),
            None
        );
    }

    #[test]
    fn resolve_request_base_url_accepts_forwarded_loopback_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::FORWARDED,
            HeaderValue::from_static("proto=https;host=tauri.localhost:3000"),
        );
        assert_eq!(
            resolve_request_base_url(&headers, "http://127.0.0.1:4321", None),
            Some("https://tauri.localhost:3000".to_string())
        );
    }

    #[test]
    fn resolve_request_base_url_uses_loopback_fallback_without_request_host() {
        let headers = HeaderMap::new();
        assert_eq!(
            resolve_request_base_url(&headers, "http://127.0.0.1:4321", None),
            Some("http://127.0.0.1:4321".to_string())
        );
    }

    #[test]
    fn resolve_request_base_url_prefers_configured_public_base_url() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("127.0.0.1:4455"));
        headers.insert(
            header::FORWARDED,
            HeaderValue::from_static("proto=https;host=proxy.example"),
        );
        assert_eq!(
            resolve_request_base_url(
                &headers,
                "http://127.0.0.1:4321",
                Some("https://proxy.example/ctx"),
            ),
            Some("https://proxy.example/ctx".to_string())
        );
    }

    #[test]
    fn public_route_url_preserves_path_prefix_and_query() {
        assert_eq!(
            public_route_url(
                "https://proxy.example/ctx",
                "/sessions/web/sess-1/view?token=stream-token",
            ),
            Some(
                "https://proxy.example/ctx/sessions/web/sess-1/view?token=stream-token".to_string(),
            )
        );
    }

    #[test]
    fn public_websocket_url_preserves_path_prefix_and_query() {
        assert_eq!(
            public_websocket_url(
                "https://proxy.example/ctx",
                "/sessions/web/sess-1/signal?token=signal-token",
            ),
            Some(
                "wss://proxy.example/ctx/sessions/web/sess-1/signal?token=signal-token".to_string(),
            )
        );
    }
}
