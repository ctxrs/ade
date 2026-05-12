use crate::daemon::AppState;

use super::ProviderCacheStats;

pub(super) async fn collect_provider_cache_stats(state: &AppState) -> ProviderCacheStats {
    let adapters = state.providers.adapters.lock().await.len();
    let statuses = state.providers.statuses.lock().await.len();
    let options_cache = state
        .providers
        .with_provider_options_cache(|cache| cache.len())
        .await;
    let verify_cache = state
        .providers
        .with_provider_verify_cache(|cache| cache.len())
        .await;
    let usage_cache = state
        .providers
        .with_provider_usage_cache(|cache| cache.len())
        .await;
    let installs = state.providers.installs.lock().await.len();

    ProviderCacheStats {
        adapters,
        statuses,
        options_cache,
        verify_cache,
        usage_cache,
        installs,
    }
}
