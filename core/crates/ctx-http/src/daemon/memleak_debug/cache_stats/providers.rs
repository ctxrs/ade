use crate::daemon::AppState;

use super::ProviderCacheStats;

pub(super) async fn collect_provider_cache_stats(state: &AppState) -> ProviderCacheStats {
    let adapters = state.providers.adapters.lock().await.len();
    let statuses = state.providers.statuses.lock().await.len();
    let options_cache = state.providers.options_cache.lock().await.len();
    let verify_cache = state.providers.verify_cache.lock().await.len();
    let usage_cache = state.providers.usage_cache.lock().await.len();
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
