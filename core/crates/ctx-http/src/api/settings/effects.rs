use super::*;

use crate::daemon::{provider_guard, provider_restart, resource_governance, tool_cgroup};
use ctx_observability::telemetry::TelemetryConfig;

pub(super) async fn public_settings_for_response(
    state: &Arc<AppState>,
    settings: &user_settings::Settings,
) -> user_settings::PublicSettings {
    let mut public = ctx_settings_service::to_public(settings);
    public.resource_governance = resource_governance::build_public_settings(state, settings).await;
    public.tool_limits = tool_cgroup::build_public_settings(state, settings).await;
    public
}

pub(super) async fn apply_settings_side_effects(
    state: &Arc<AppState>,
    settings: &user_settings::Settings,
) {
    let mut telemetry_cfg = TelemetryConfig::default();
    if let Some(telemetry) = settings.telemetry.as_ref() {
        telemetry_cfg.enabled = telemetry.enabled;
        if !telemetry.endpoint.trim().is_empty() {
            telemetry_cfg.endpoint = telemetry.endpoint.clone();
        }
    }
    state.telemetry.telemetry.update_config(telemetry_cfg).await;
    let perf_enabled = settings
        .telemetry
        .as_ref()
        .map(|t| t.enabled)
        .unwrap_or(true);
    state
        .telemetry
        .perf_telemetry
        .update_remote_enabled(perf_enabled)
        .await;
    if let Err(err) = resource_governance::apply_settings(state, settings).await {
        tracing::warn!("failed to apply resource governance settings: {err:#}");
    }
    if let Err(err) = provider_guard::apply_settings(state, settings).await {
        tracing::warn!("failed to apply provider guard settings: {err:#}");
    }
    if let Err(err) = provider_restart::apply_settings(state, settings).await {
        tracing::warn!("failed to apply provider restart settings: {err:#}");
    }
    if let Err(err) = tool_cgroup::apply_settings(state, settings).await {
        tracing::warn!("failed to apply tool cgroup settings: {err:#}");
    }
}
