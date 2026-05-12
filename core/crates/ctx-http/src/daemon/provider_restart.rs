use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{broadcast, Mutex};

use ctx_providers::adapters::ProviderRestartMode;

use crate::daemon::AppState;
use ctx_settings_model::{ProviderRestartSettings, ResourceGovernanceMode, Settings};

mod notices;
mod processes;

use notices::notify_sessions;
use processes::{list_provider_processes, signal_pids};

pub use ctx_provider_runtime::provider_restart::{
    compute_effective_limits, ProviderRestartConfig, ProviderRestartEvent, ProviderRestartLimits,
    ProviderRestartRuntime, ResourceGovernanceMode as RestartGovernanceMode,
};

pub async fn apply_settings(state: &AppState, settings: &Settings) -> Result<()> {
    let cfg = settings.provider_restart.clone().unwrap_or_default();
    ctx_provider_runtime::provider_restart::apply_settings(state, &map_config(&cfg)).await
}

pub fn spawn_provider_restart(state: Arc<AppState>) {
    ctx_provider_runtime::provider_restart::spawn_provider_restart(state);
}

#[async_trait::async_trait]
impl ctx_provider_runtime::provider_restart::ProviderRestartHost for AppState {
    fn provider_restart_runtime(&self) -> &Mutex<ProviderRestartRuntime> {
        &self.providers.restart
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
        let provider_processes = list_provider_processes(self).await;
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

    async fn restart_provider(&self, provider_id: &str, pid: u32) {
        let adapter = self.providers.provider_adapter(provider_id).await;
        let mut needs_kill = true;
        if let Some(adapter) = adapter {
            match adapter
                .restart(
                    "provider restart: sustained memory usage",
                    ProviderRestartMode::Immediate,
                )
                .await
            {
                Ok(()) => needs_kill = false,
                Err(err) => {
                    tracing::warn!(provider_id, pid, "provider restart hook failed: {err:#}")
                }
            }
        } else {
            tracing::warn!(
                provider_id,
                pid,
                "provider restart failed: provider adapter missing"
            );
        }

        if needs_kill {
            let killed = signal_pids(&[pid], processes::PROCESS_KILL_SIGNAL);
            if killed == 0 {
                tracing::warn!(provider_id, pid, "provider restart failed to kill process");
            }
        }
    }

    async fn on_provider_restart_notice(state: &Arc<Self>, event: ProviderRestartEvent) {
        notify_sessions(state, &event).await;
    }
}

fn map_config(settings: &ProviderRestartSettings) -> ProviderRestartConfig {
    ProviderRestartConfig {
        enabled: settings.enabled,
        mode: Some(match settings.mode {
            ResourceGovernanceMode::Auto => RestartGovernanceMode::Auto,
            ResourceGovernanceMode::Custom => RestartGovernanceMode::Custom,
        }),
        memory_high_mb: settings.memory_high_mb,
        memory_max_mb: settings.memory_max_mb,
        interval_ms: settings.interval_ms,
        grace_period_ms: settings.grace_period_ms,
    }
}
