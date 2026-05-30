use crate::daemon::workspaces::WorkspaceCacheDebugStatsHost;

use super::WorkspaceCacheStats;

pub(super) async fn collect_workspace_cache_stats(
    workspaces: &WorkspaceCacheDebugStatsHost,
) -> WorkspaceCacheStats {
    workspaces.cache_debug_stats().await
}
