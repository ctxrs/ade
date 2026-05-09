use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{broadcast, Mutex};

use crate::daemon::AppState;
use ctx_settings_model::{ProviderGuardSettings, ResourceGovernanceMode, Settings};

mod events;
mod processes;
mod snapshot;

pub use ctx_provider_runtime::provider_guard::{
    compute_effective_limits, ProviderGuardConfig, ProviderGuardLimits, ProviderGuardRuntime,
    ResourceGovernanceMode as GuardMode,
};

pub async fn apply_settings(state: &AppState, settings: &Settings) -> Result<()> {
    let cfg = settings.provider_guard.clone().unwrap_or_default();
    ctx_provider_runtime::provider_guard::apply_settings(state, &map_config(&cfg)).await
}

pub fn spawn_provider_guard(state: Arc<AppState>) {
    ctx_provider_runtime::provider_guard::spawn_provider_guard(state);
}

#[async_trait::async_trait]
impl ctx_provider_runtime::provider_guard::ProviderGuardHost for AppState {
    fn provider_guard_runtime(&self) -> &Mutex<ProviderGuardRuntime> {
        &self.providers.guard
    }

    fn subscribe_shutdown(&self) -> broadcast::Receiver<()> {
        self.core.shutdown_tx.subscribe()
    }

    async fn system_snapshot(&self) -> ctx_provider_runtime::provider_guard::SystemSnapshot {
        let (system, _disks, _cache_age_ms) = {
            let mut sampler = self.telemetry.resource_sampler.lock().await;
            sampler.system_snapshot()
        };
        ctx_provider_runtime::provider_guard::SystemSnapshot {
            memory_total_bytes: system.memory_total_bytes,
            memory_used_bytes: system.memory_used_bytes,
        }
    }

    async fn provider_memory_snapshot(
        &self,
    ) -> Vec<ctx_provider_runtime::provider_guard::ProviderMemorySample> {
        let provider_processes = processes::list_provider_processes(self).await;
        let samples = {
            let mut sampler = self.telemetry.resource_sampler.lock().await;
            sampler.provider_memory_snapshot(&provider_processes)
        };
        samples
            .into_iter()
            .map(
                |sample| ctx_provider_runtime::provider_guard::ProviderMemorySample {
                    provider_id: sample.provider_id,
                    label: sample.label,
                    pid: sample.pid,
                    memory_bytes: sample.memory_bytes,
                    tool_memory_bytes: sample.tool_memory_bytes,
                },
            )
            .collect()
    }

    async fn on_provider_guard_event(
        state: &Arc<Self>,
        event: ctx_provider_runtime::provider_guard::ProviderGuardEvent,
    ) {
        events::handle_provider_guard_event(state, &event).await;
    }
}

fn map_config(settings: &ProviderGuardSettings) -> ProviderGuardConfig {
    ProviderGuardConfig {
        enabled: settings.enabled,
        mode: Some(match settings.mode {
            ResourceGovernanceMode::Auto => GuardMode::Auto,
            ResourceGovernanceMode::Custom => GuardMode::Custom,
        }),
        memory_high_mb: settings.memory_high_mb,
        memory_max_mb: settings.memory_max_mb,
        interval_ms: settings.interval_ms,
        grace_period_ms: settings.grace_period_ms,
    }
}
