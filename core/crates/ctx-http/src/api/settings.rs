use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;

use crate::daemon::AppState;
use crate::provider_guard;
use crate::provider_restart;
use crate::resource_governance;
use crate::settings as user_settings;
use crate::telemetry::TelemetryConfig;
use crate::tool_cgroup;

pub(super) async fn get_settings(
    State(state): State<Arc<AppState>>,
) -> Result<Json<user_settings::PublicSettings>, StatusCode> {
    let settings = user_settings::load_settings(&state.data_root).await;
    let mut public = user_settings::to_public(&settings);
    public.resource_governance =
        resource_governance::build_public_settings(&state, &settings).await;
    public.tool_limits = tool_cgroup::build_public_settings(&state, &settings).await;
    Ok(Json(public))
}

pub(super) async fn update_settings(
    State(state): State<Arc<AppState>>,
    Json(req): Json<user_settings::UpdateSettingsReq>,
) -> Result<Json<user_settings::PublicSettings>, StatusCode> {
    let current = user_settings::load_settings(&state.data_root).await;
    let next = user_settings::apply_update(current, req);
    user_settings::save_settings(&state.data_root, &next)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut telemetry_cfg = TelemetryConfig::default();
    if let Some(telemetry) = next.telemetry.as_ref() {
        telemetry_cfg.enabled = telemetry.enabled;
        if !telemetry.endpoint.trim().is_empty() {
            telemetry_cfg.endpoint = telemetry.endpoint.clone();
        }
    }
    state.telemetry.update_config(telemetry_cfg).await;
    let perf_enabled = next.telemetry.as_ref().map(|t| t.enabled).unwrap_or(true);
    state
        .perf_telemetry
        .update_remote_enabled(perf_enabled)
        .await;
    if let Err(err) = resource_governance::apply_settings(&state, &next).await {
        tracing::warn!("failed to apply resource governance settings: {err:#}");
    }
    if let Err(err) = provider_guard::apply_settings(&state, &next).await {
        tracing::warn!("failed to apply provider guard settings: {err:#}");
    }
    if let Err(err) = provider_restart::apply_settings(&state, &next).await {
        tracing::warn!("failed to apply provider restart settings: {err:#}");
    }
    if let Err(err) = tool_cgroup::apply_settings(&state, &next).await {
        tracing::warn!("failed to apply tool cgroup settings: {err:#}");
    }
    let mut public = user_settings::to_public(&next);
    public.resource_governance = resource_governance::build_public_settings(&state, &next).await;
    public.tool_limits = tool_cgroup::build_public_settings(&state, &next).await;
    Ok(Json(public))
}
