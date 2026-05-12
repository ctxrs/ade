use anyhow::Result;
use ctx_resource_utilization::resource_governance::{
    apply_limits, compute_effective_limits, public_settings, status_for, ResourceGovernanceRuntime,
};

use crate::daemon::AppState;
use ctx_settings_model::{
    PublicResourceGovernanceSettings, ResourceGovernanceStatusState, Settings,
};

pub async fn apply_settings(state: &AppState, settings: &Settings) -> Result<()> {
    let cfg = settings.resource_governance.clone().unwrap_or_default();
    let (system, _disks, _cache_age_ms) = {
        let mut sampler = state.telemetry.resource_sampler.lock().await;
        sampler.system_snapshot()
    };
    let cpu_count = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let effective = compute_effective_limits(&cfg, &system, cpu_count);

    let mut runtime = if let Some(limits) = effective.as_ref() {
        let has_running_children = has_running_children(state).await;
        apply_limits(std::process::id(), limits, has_running_children).await
    } else {
        ResourceGovernanceRuntime::default()
    };

    if !cfg.enabled {
        runtime.last_state = ResourceGovernanceStatusState::Disabled;
        runtime.last_message = None;
        runtime.last_applied = None;
        runtime.requires_restart = false;
    }

    let mut guard = state.telemetry.resource_governance.lock().await;
    *guard = runtime;
    Ok(())
}

pub async fn build_public_settings(
    state: &AppState,
    settings: &Settings,
) -> Option<PublicResourceGovernanceSettings> {
    let cfg = settings.resource_governance.as_ref()?;
    let (system, _disks, _cache_age_ms) = {
        let mut sampler = state.telemetry.resource_sampler.lock().await;
        sampler.system_snapshot()
    };
    let cpu_count = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let effective = compute_effective_limits(cfg, &system, cpu_count);
    let runtime = state.telemetry.resource_governance.lock().await.clone();
    let status = status_for(cfg.enabled, effective.as_ref(), &runtime);
    Some(public_settings(cfg, effective.as_ref(), status))
}

async fn has_running_children(state: &AppState) -> bool {
    state.providers.has_running_provider_processes().await
        || state.transport.terminals.has_running().await
}
