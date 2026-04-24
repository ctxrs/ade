use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::Utc;
use ctx_core::provider_ids::canonical_provider_id;

use ctx_providers::adapters::{
    ProviderAdapter, ProviderRestartMode, ProviderSessionSweepConfig, ProviderSessionSweepStats,
};

use super::{AppState, CacheSweepConfig};

pub(super) fn spawn_cache_sweeper(state: Arc<AppState>) {
    let config = CacheSweepConfig::from_env();
    tokio::spawn(async move {
        let mut shutdown_rx = state.core.shutdown_tx.subscribe();
        loop {
            tokio::select! {
                _ = tokio::time::sleep(config.interval) => {
                    let stats = state.sweep_idle_caches(Instant::now(), config).await;
                    if stats.total_evicted() > 0 {
                        tracing::info!(
                            session_head_evicted = stats.session_head_evicted,
                            session_meta_evicted = stats.session_meta_evicted,
                            schedulers_evicted = stats.schedulers_evicted,
                            broadcasters_evicted = stats.broadcasters_evicted,
                            session_event_heads_evicted = stats.session_event_heads_evicted,
                            file_completions_evicted = stats.file_completions_evicted,
                            workspace_file_completions_evicted =
                                stats.workspace_file_completions_evicted,
                            git_status_evicted = stats.git_status_evicted,
                            workspace_snapshot_evicted = stats.workspace_snapshot_evicted,
                            workspace_heads_evicted = stats.workspace_heads_evicted,
                            worktree_bootstrap_evicted = stats.worktree_bootstrap_evicted,
                            workspace_stores_evicted = stats.workspace_stores_evicted,
                            "cache sweep completed"
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

pub(super) fn spawn_provider_worker_sweeper(state: Arc<AppState>) {
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

const DEFAULT_ENDPOINT_MODEL_SWEEP_INTERVAL: Duration = Duration::from_secs(60 * 60 * 6);

fn endpoint_model_sweep_interval() -> Duration {
    std::env::var("CTX_ENDPOINT_MODEL_SWEEP_SECS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_ENDPOINT_MODEL_SWEEP_INTERVAL)
}

async fn refresh_stale_selected_endpoint_model_catalogs(
    state: &Arc<AppState>,
) -> (usize, usize, HashSet<String>) {
    let provider_ids = {
        let statuses = state.providers.statuses.lock().await;
        statuses.keys().cloned().collect::<Vec<_>>()
    };
    let now = Utc::now();
    let mut refreshed = 0usize;
    let mut failed = 0usize;
    let mut refreshed_provider_ids = HashSet::new();

    for provider_id in provider_ids {
        let Ok(config) =
            ctx_harness_sources::get_provider_source_config(&state.core.data_root, &provider_id)
                .await
        else {
            continue;
        };
        if config.selected_source_kind != ctx_harness_sources::HarnessSourceKind::Endpoint {
            continue;
        }
        let Some(selected_endpoint_id) = config.selected_endpoint_id.as_deref() else {
            continue;
        };
        let Some(endpoint) = config
            .endpoints
            .iter()
            .find(|candidate| candidate.id == selected_endpoint_id)
        else {
            continue;
        };
        if !ctx_harness_sources::endpoint_model_catalog_is_stale(endpoint, now) {
            continue;
        }

        match ctx_harness_sources::refresh_provider_endpoint_model_catalog(
            &state.core.data_root,
            &provider_id,
            selected_endpoint_id,
        )
        .await
        {
            Ok(_) => {
                refreshed += 1;
                refreshed_provider_ids.insert(provider_id);
            }
            Err(err) => {
                failed += 1;
                tracing::warn!(
                    provider_id = provider_id,
                    endpoint_id = selected_endpoint_id,
                    err = %err,
                    "endpoint model catalog refresh failed"
                );
            }
        }
    }

    (refreshed, failed, refreshed_provider_ids)
}

pub(super) fn spawn_endpoint_model_catalog_sweeper(state: Arc<AppState>) {
    let interval = endpoint_model_sweep_interval();
    tokio::spawn(async move {
        let mut shutdown_rx = state.core.shutdown_tx.subscribe();
        loop {
            tokio::select! {
                _ = tokio::time::sleep(interval) => {
                    let (refreshed, failed, refreshed_provider_ids) =
                        refresh_stale_selected_endpoint_model_catalogs(&state).await;

                    if !refreshed_provider_ids.is_empty() {
                        state
                            .providers
                            .options_cache
                            .lock()
                            .await
                            .retain(|cache_key, _| {
                                !refreshed_provider_ids
                                    .iter()
                                    .any(|provider_id| cache_key_matches_provider(cache_key, provider_id))
                            });
                    }

                    if refreshed > 0 || failed > 0 {
                        tracing::info!(
                            refreshed_endpoint_catalogs = refreshed,
                            failed_endpoint_catalog_refreshes = failed,
                            "endpoint model catalog sweep completed"
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

fn cache_key_matches_provider(cache_key: &str, provider_id: &str) -> bool {
    let provider_id = canonical_provider_id(provider_id);
    cache_key
        .rsplit_once('/')
        .is_some_and(|(_, key_provider)| canonical_provider_id(key_provider) == provider_id)
}

pub(crate) async fn collect_provider_adapters_for_shutdown(
    state: &Arc<AppState>,
) -> Vec<(String, Arc<dyn ProviderAdapter>)> {
    let mut adapters = {
        let map = state.providers.adapters.lock().await;
        map.iter()
            .map(|(id, adapter)| (id.clone(), Arc::clone(adapter)))
            .collect::<Vec<_>>()
    };
    let target_adapters = {
        let map = state.providers.target_adapters.lock().await;
        map.iter()
            .map(|(id, adapter)| (id.clone(), Arc::clone(adapter)))
            .collect::<Vec<_>>()
    };
    adapters.extend(target_adapters);
    adapters
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

pub(crate) async fn shutdown_shared_substrate(
    state: &Arc<AppState>,
    reason: &str,
) -> Result<Option<ctx_avf_linux_runtime::SubstrateLifecycleRecord>> {
    let Some(record) = state
        .execution
        .harness
        .save_or_stop_selected_shared_substrate()
        .await?
    else {
        return Ok(None);
    };

    tracing::info!(
        shutdown_reason = reason,
        substrate = ?record.substrate,
        shutdown_outcome = ?record.shutdown_outcome,
        shutdown_detail = ?record.shutdown_reason,
        save_error_present = record.save_error_present,
        saved_state_written_on_shutdown = record.saved_state_written_on_shutdown,
        simulated = record.simulated,
        "shared substrate save-or-stop requested for daemon shutdown"
    );
    Ok(Some(record))
}

async fn trigger_daemon_shutdown(state: Arc<AppState>, reason: &str) {
    tracing::info!("daemon shutdown requested: {reason}");
    shutdown_provider_adapters(&state, reason).await;
    if let Err(err) = shutdown_shared_substrate(&state, reason).await {
        tracing::warn!("failed to save-or-stop shared substrate during daemon shutdown: {err:#}");
    }
    let _ = state.core.shutdown_tx.send(());
}

pub(super) fn spawn_process_shutdown_listener(state: Arc<AppState>) {
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            let mut sigterm =
                match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                    Ok(signal) => signal,
                    Err(err) => {
                        tracing::warn!("failed to register SIGTERM handler: {err:#}");
                        return;
                    }
                };
            tokio::select! {
                result = tokio::signal::ctrl_c() => {
                    if let Err(err) = result {
                        tracing::warn!("failed to listen for ctrl_c: {err:#}");
                        return;
                    }
                    trigger_daemon_shutdown(state, "ctrl_c").await;
                }
                _ = sigterm.recv() => {
                    trigger_daemon_shutdown(state, "sigterm").await;
                }
            }
        }

        #[cfg(not(unix))]
        {
            if let Err(err) = tokio::signal::ctrl_c().await {
                tracing::warn!("failed to listen for ctrl_c: {err:#}");
                return;
            }
            trigger_daemon_shutdown(state, "ctrl_c").await;
        }
    });
}
