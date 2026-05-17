use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;

use crate::api::errors::ApiErrorResp;
use ctx_daemon::daemon::{
    BeginUpdateDrainRouteRequest, ExecutionHandle, MaintenanceRouteError,
    MaintenanceRouteErrorKind, ReleaseUpdateDrainRouteRequest,
};

pub(in crate::api) async fn begin_update_drain(
    State(execution): State<ExecutionHandle>,
    Json(req): Json<BeginUpdateDrainRouteRequest>,
) -> Result<Json<ctx_daemon::daemon::BeginUpdateDrainRouteResult>, (StatusCode, Json<ApiErrorResp>)>
{
    let result = execution
        .begin_update_drain_for_route(req)
        .await
        .map_err(maintenance_route_error)?;
    Ok(Json(result))
}

pub(in crate::api) async fn release_update_drain(
    State(execution): State<ExecutionHandle>,
    Json(req): Json<ReleaseUpdateDrainRouteRequest>,
) -> Result<Json<ctx_daemon::daemon::ReleaseUpdateDrainRouteResult>, (StatusCode, Json<ApiErrorResp>)>
{
    let result = execution
        .release_update_drain_for_route(req)
        .await
        .map_err(maintenance_route_error)?;
    Ok(Json(result))
}

pub(super) fn maintenance_route_error(
    error: MaintenanceRouteError,
) -> (StatusCode, Json<ApiErrorResp>) {
    let status = match error.kind() {
        MaintenanceRouteErrorKind::BadRequest => StatusCode::BAD_REQUEST,
        MaintenanceRouteErrorKind::Conflict => StatusCode::CONFLICT,
        MaintenanceRouteErrorKind::Forbidden => StatusCode::FORBIDDEN,
        MaintenanceRouteErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(ApiErrorResp {
            error: error.message().to_string(),
        }),
    )
}
