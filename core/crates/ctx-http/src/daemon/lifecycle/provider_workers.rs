use std::collections::HashSet;
use std::sync::Arc;

use ctx_providers::adapters::{
    ProviderAdapter, ProviderRestartMode, ProviderSessionSweepConfig, ProviderSessionSweepStats,
};

use crate::daemon::AppState;

pub(crate) async fn sweep_provider_workers_once(
    state: &Arc<AppState>,
    config: ProviderSessionSweepConfig,
) -> ProviderSessionSweepStats {
    let mut stats = ProviderSessionSweepStats::default();
    let mut seen = HashSet::<usize>::new();
    for (_, adapter) in collect_provider_adapters_for_shutdown(state).await {
        let identity = (Arc::as_ptr(&adapter) as *const ()) as usize;
        if !seen.insert(identity) {
            continue;
        }
        match adapter.reap_idle_sessions(config).await {
            Ok(adapter_stats) => {
                stats.reaped += adapter_stats.reaped;
                stats.skipped_busy += adapter_stats.skipped_busy;
                stats.dead_removed += adapter_stats.dead_removed;
                stats.status_errors += adapter_stats.status_errors;
            }
            Err(err) => {
                stats.status_errors += 1;
                tracing::debug!(err = %err, "provider worker sweep failed");
            }
        }
    }
    stats
}

pub(in crate::daemon) fn spawn_provider_worker_sweeper(state: Arc<AppState>) {
    let config = ProviderSessionSweepConfig::from_env();
    tokio::spawn(async move {
        let mut shutdown_rx = state.core.shutdown_tx.subscribe();
        loop {
            tokio::select! {
                _ = tokio::time::sleep(config.interval) => {
                    let stats = sweep_provider_workers_once(&state, config).await;
                    if stats.total_actions() > 0 || stats.skipped_busy > 0 || stats.status_errors > 0 {
                        tracing::info!(
                            reaped = stats.reaped,
                            dead_removed = stats.dead_removed,
                            skipped_busy = stats.skipped_busy,
                            status_errors = stats.status_errors,
                            "provider worker sweep completed"
                        );
                    }
                }
                _ = shutdown_rx.recv() => {
                    break;
                }
            }
        }
    });
}

pub(crate) async fn collect_provider_adapters_for_shutdown(
    state: &Arc<AppState>,
) -> Vec<(String, Arc<dyn ProviderAdapter>)> {
    state.providers.all_provider_adapter_entries().await
}

pub(crate) async fn shutdown_provider_adapters(state: &Arc<AppState>, reason: &str) {
    for (id, adapter) in collect_provider_adapters_for_shutdown(state).await {
        if let Err(err) = adapter
            .restart(reason, ProviderRestartMode::Immediate)
            .await
        {
            tracing::debug!("failed to stop provider adapter {id} during daemon shutdown: {err:#}");
        }
    }
}
