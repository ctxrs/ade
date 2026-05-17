use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use ctx_daemon::daemon::{ApplyAppImageUpdateRequest, ApplyAppImageUpdateResult, CoreHandle};

use crate::api::errors::ApiErrorResp;
use crate::api::updates::update_route_error;

pub(in crate::api) async fn apply_appimage_update(
    State(core): State<CoreHandle>,
    Json(req): Json<ApplyAppImageUpdateRequest>,
) -> Result<Json<ApplyAppImageUpdateResult>, (StatusCode, Json<ApiErrorResp>)> {
    core.apply_appimage_update(env!("CARGO_PKG_VERSION"), req)
        .await
        .map(Json)
        .map_err(update_route_error)
}
