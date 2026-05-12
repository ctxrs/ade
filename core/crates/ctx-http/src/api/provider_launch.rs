use std::sync::Arc;
use std::time::Duration;

mod errors;
mod handlers;
mod provider_options_response;
mod runtime_probe;

use errors::{provider_install_error_response, workspace_execution_settings_error_json};
pub(in crate::api) use handlers::*;
use provider_options_response::*;
use runtime_probe::{prepare_provider_runtime_probe, PreparedProviderRuntimeProbeError};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::Json;
use chrono::Utc;
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use super::errors::ApiErrorResp;
use super::provider_probe_auth::provider_auth_mode;
use super::providers::{install_target_for_workspace, provider_status_for_target};
use super::redact_json_value;
use crate::daemon::AppState;
use ctx_harness_sources as harness_sources;
use ctx_harness_sources::{HarnessEndpointVerificationStatus, HarnessSourceKind};
use ctx_observability::logs;
use ctx_provider_install::install_state::{
    InstallId, InstallInfo, InstallProgressEvent, InstallTarget,
};
use ctx_provider_runtime::model_preferences::inject_preferred_model_id;
use ctx_provider_runtime::provider_auth::{
    selected_endpoint_from_harness_config, selected_endpoint_record_from_harness_config,
};
use ctx_provider_runtime::provider_cache::workspace_provider_cache_key;
use ctx_provider_runtime::provider_launch::config::{
    load_managed_agent_server_config_with_error, load_provider_source_config_with_error,
};
use ctx_provider_runtime::provider_launch::install as provider_launch_install;
use ctx_provider_runtime::provider_launch::models::{
    endpoint_catalog_runtime_probe_failure, endpoint_catalog_verify_outcome,
};
use ctx_provider_runtime::provider_launch::options::{
    endpoint_supports_model_catalog_verify, provider_options_cache_entry_is_authoritative,
    provider_supports_runtime_model_catalog,
};
use ctx_provider_runtime::provider_launch::probe;
use ctx_provider_runtime::provider_launch::probe_error::classify_probe_error;
use ctx_provider_runtime::provider_usability::{
    provider_status_is_usable, provider_status_unusable_reason,
};
use ctx_providers::crp::probe_crp_models;

#[derive(Debug, Deserialize)]
pub(super) struct InstallTargetQuery {
    target: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct InstallStartResponse {
    provider_id: String,
    install_id: InstallId,
    target: InstallTarget,
}

#[derive(Debug, Deserialize)]
pub(super) struct AuthenticateProviderReq {
    #[serde(default)]
    method_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct ProviderAuthCheckResp {
    provider_id: String,
    workspace_id: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_required: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    checked_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}
