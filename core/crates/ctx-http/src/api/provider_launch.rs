use std::time::Duration;

mod common;
mod errors;
mod handlers;

use common::parse_workspace_id;
use errors::{
    provider_install_error_response, provider_launch_config_error_response,
    workspace_execution_settings_error_json,
};
pub(in crate::api) use handlers::*;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::Json;
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};

use super::errors::ApiErrorResp;
use crate::daemon::providers::parse_provider_install_target;
use crate::daemon::ProvidersHandle;
use ctx_observability::logs;
use ctx_provider_install::install_state::{
    InstallId, InstallInfo, InstallProgressEvent, InstallTarget,
};

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

impl From<crate::daemon::providers::ProviderAuthCheckSnapshot> for ProviderAuthCheckResp {
    fn from(value: crate::daemon::providers::ProviderAuthCheckSnapshot) -> Self {
        Self {
            provider_id: value.provider_id,
            workspace_id: value.workspace_id,
            status: value.status,
            auth_required: value.auth_required,
            checked_at: value.checked_at,
            message: value.message,
        }
    }
}
