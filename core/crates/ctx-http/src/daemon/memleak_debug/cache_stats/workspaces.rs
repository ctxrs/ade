use crate::daemon::AppState;

use super::WorkspaceCacheStats;

pub(super) async fn collect_workspace_cache_stats(state: &AppState) -> WorkspaceCacheStats {
    state.workspace_cache_debug_stats().await
}
