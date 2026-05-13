use crate::daemon::AppState;

use super::SessionCacheStats;

pub(super) async fn collect_session_cache_stats(state: &AppState) -> SessionCacheStats {
    state.session_cache_debug_stats().await
}
