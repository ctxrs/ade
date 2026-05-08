use anyhow::Result;
use ctx_resource_utilization::tool_limits::{
    apply_limits, compute_effective_limits, public_settings, ToolLimitsApplyOutcome,
};

#[cfg(target_os = "linux")]
pub const TOOL_SLICE_UNIT: &str = ctx_resource_utilization::tool_limits::TOOL_SLICE_UNIT;

use crate::daemon::AppState;
use crate::settings::{PublicToolLimitsSettings, Settings};

pub async fn build_public_settings(
    state: &AppState,
    settings: &Settings,
) -> Option<PublicToolLimitsSettings> {
    let cfg = settings.tool_limits.as_ref()?;
    let (system, _disks, _cache_age_ms) = {
        let mut sampler = state.telemetry.resource_sampler.lock().await;
        sampler.system_snapshot()
    };
    let effective = compute_effective_limits(cfg, &system);
    Some(public_settings(cfg, effective.as_ref()))
}

pub async fn apply_settings(state: &AppState, settings: &Settings) -> Result<()> {
    let cfg = settings.tool_limits.clone().unwrap_or_default();
    if !cfg.enabled {
        return Ok(());
    }

    let (system, _disks, _cache_age_ms) = {
        let mut sampler = state.telemetry.resource_sampler.lock().await;
        sampler.system_snapshot()
    };
    let Some(limits) = compute_effective_limits(&cfg, &system) else {
        return Ok(());
    };

    match apply_limits(&limits).await? {
        ToolLimitsApplyOutcome::Applied => {}
        ToolLimitsApplyOutcome::Unsupported => {
            tracing::warn!("tool cgroup limits are not supported on this host");
        }
    }
    Ok(())
}
