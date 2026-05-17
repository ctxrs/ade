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
use serde::Deserialize;

use ctx_daemon::daemon::providers::{
    AuthenticateProviderForWorkspaceRouteBody, AuthenticateProviderForWorkspaceRouteRequest,
    ProviderAuthCheckRouteError, ProviderAuthCheckRouteErrorStatus, ProviderAuthCheckRouteResponse,
    ProviderInstallInfo, ProviderInstallProgressEvent, ProviderInstallStartRouteResponse,
    ProviderInstallStatusOnlyRouteError, ProviderInstallStatusesRouteRequest,
    ProviderInstallStatusesRouteResponse, VerifyProviderForWorkspaceRouteRequest,
};
use ctx_daemon::daemon::ProvidersHandle;
use ctx_observability::logs;

#[derive(Debug, Deserialize)]
pub(super) struct RawInstallTargetQuery {
    target: Option<String>,
}
