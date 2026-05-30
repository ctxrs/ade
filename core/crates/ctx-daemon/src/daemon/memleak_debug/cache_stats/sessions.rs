use crate::daemon::state::SessionRuntime;

use super::SessionCacheStats;

pub(super) async fn collect_session_cache_stats(sessions: &SessionRuntime) -> SessionCacheStats {
    sessions.cache_debug_stats().await
}
