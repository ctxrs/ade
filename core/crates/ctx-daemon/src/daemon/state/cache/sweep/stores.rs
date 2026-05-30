use super::super::{CacheSweepConfig, CacheSweepHost, CacheSweepStats};

impl CacheSweepHost {
    pub(super) async fn sweep_workspace_stores(
        &self,
        config: CacheSweepConfig,
        stats: &mut CacheSweepStats,
    ) {
        stats.workspace_stores_evicted = self
            .workspace_stores
            .evict_idle_workspace_stores(config.workspace_ttl)
            .await;
    }
}
