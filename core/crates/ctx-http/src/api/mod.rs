use std::collections::HashMap;
use std::path::Path as StdPath;
use std::sync::Arc;
use std::time::Instant;

use axum::body::{Body, Bytes};
use axum::extract::{Extension, MatchedPath, Path, Query, State};
use axum::http::header;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::Engine;
use opentelemetry::trace::SpanKind;
use opentelemetry::KeyValue;
use serde::{Deserialize, Serialize};
use url::Url;

pub(crate) mod artifacts;
mod auth;
mod demo;
mod diagnostics;
pub(crate) mod errors;
mod execution;
mod health;
mod logs_api;
mod mcp_context;
mod mcp_scope;
mod merge_queue_api;
mod mobile_access;
mod mobile_scopes;
mod org_policy;
mod perf;
mod provider_launch;
pub(crate) mod providers;
mod repo;
mod request_base;
mod resource_utilization;
mod router;
mod routes;
mod run_archive;
pub(crate) mod sessions;
mod settings;
pub(crate) mod shared;
pub(crate) mod tasks;
mod telemetry;
mod terminals;
mod title_generation;
mod updates;
mod web_sessions;
pub(crate) mod workspace_provider_model_preferences;
mod workspaces;
mod ws;

#[cfg(test)]
pub(crate) use artifacts::open_canonical_session_artifact_file;

use artifacts::*;
use diagnostics::*;
use execution::*;
use health::*;
use logs_api::*;
use mcp_context::*;
use mcp_scope::*;
use merge_queue_api::*;
use mobile_access::*;
use mobile_scopes::*;
use org_policy::*;
use providers::*;
use repo::*;
use resource_utilization::*;
use run_archive::*;
use sessions::*;
use settings::*;
use tasks::*;
use telemetry::*;
use terminals::*;
use title_generation::*;
use updates::*;
use web_sessions::*;
use workspaces::*;

use request_base::{public_route_url, public_websocket_url, resolve_request_base_url};
pub use router::router;

use auth::{
    generate_mobile_api_token, generate_pairing_token, hash_api_token, hash_pairing_token,
    load_mobile_auth_context_for_profile, MobileAuthContext,
};
use demo::*;
use errors::ApiErrorResp;
use ws::{
    dictation_livekit_stream_ws, mobile_secure_workspace_stream_ws, terminal_stream_ws,
    web_session_signal, workspace_active_snapshot_stream_ws, workspace_vcs_stream_ws,
};

use ctx_core::{ids::*, models::*};
use ctx_store::store::MobileDeviceUpsert;

use crate::daemon::AppState;
use ctx_managed_installs as installer;
use ctx_managed_installs::title_generation_local;
use ctx_observability::logs;
use ctx_observability::perf_telemetry::{PerfMetric, PerfMetricKind};
use ctx_provider_install::install_state::InstallId;
use ctx_providers::adapters::ProviderStatus;
use ctx_transport_runtime::web_sessions::{
    render_web_session_view, WebSessionInfo, WebSessionRunRequest, WebSessionRunResponse,
    WebSessionViewport,
};

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
