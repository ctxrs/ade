use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use ctx_provider_runtime::provider_cache::cache_key_matches_provider;

use crate::daemon::AppState;

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
    let provider_ids = state
        .providers
        .with_provider_statuses(|statuses| statuses.keys().cloned().collect::<Vec<_>>())
        .await;
    let now = Utc::now();
    let mut refreshed = 0usize;
    let mut failed = 0usize;
    let mut refreshed_provider_ids = HashSet::new();

    for provider_id in provider_ids {
        let config = match ctx_harness_sources::get_provider_source_config(
            &state.core.data_root,
            &provider_id,
        )
        .await
        {
            Ok(config) => config,
            Err(err) => {
                failed += 1;
                tracing::warn!(
                    provider_id = provider_id,
                    err = %err,
                    "endpoint model catalog sweep failed to load provider harness config"
                );
                continue;
            }
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

pub(in crate::daemon) fn spawn_endpoint_model_catalog_sweeper(state: Arc<AppState>) {
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
                            .with_provider_options_cache(|cache| {
                                cache.retain(|cache_key, _| {
                                !refreshed_provider_ids
                                    .iter()
                                    .any(|provider_id| cache_key_matches_provider(cache_key, provider_id))
                                });
                            })
                            .await;
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

#[cfg(test)]
mod tests;
