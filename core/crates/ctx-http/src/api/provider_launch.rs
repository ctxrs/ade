use std::time::Duration;

mod errors;
mod handlers;

use errors::provider_install_error_response;
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
    ProviderInstallStatusesRouteResponse, ProviderOptionsRouteError,
    ProviderOptionsRouteErrorStatus, ProviderOptionsRouteRequest,
    VerifyProviderForWorkspaceRouteRequest,
};
use ctx_daemon::daemon::ProvidersHandle;

#[derive(Debug, Deserialize)]
pub(super) struct RawInstallTargetQuery {
    target: Option<String>,
}
