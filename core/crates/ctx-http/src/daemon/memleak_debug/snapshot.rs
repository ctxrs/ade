use chrono::{DateTime, Utc};
use ctx_harness_runtime::HarnessRuntimeStats;
use ctx_observability::perf_telemetry::PerfTelemetryStats;
use ctx_resource_utilization::memleak_debug::{GlibcMallinfo, JemallocStats};
use ctx_store::StoreManagerStats;
use ctx_transport_runtime::terminals::TerminalManagerStats;
use ctx_transport_runtime::web_sessions::WebSessionManagerStats;
use ctx_workspace_active_snapshot::WorkspaceActiveSnapshotStats;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(super) struct MemleakDebugSnapshot {
    pub(super) occurred_at: DateTime<Utc>,
    pub(super) rss_bytes: u64,
    pub(super) thread_count: u32,
    pub(super) sessions: SessionCacheStats,
    pub(super) workspaces: WorkspaceCacheStats,
    pub(super) providers: ProviderCacheStats,
    pub(super) active_snapshot: WorkspaceActiveSnapshotStats,
    pub(super) terminals: TerminalManagerStats,
    pub(super) perf_telemetry: PerfTelemetryStats,
    pub(super) web_sessions: WebSessionManagerStats,
    pub(super) harness_runtime: HarnessRuntimeStats,
    pub(super) stores: StoreManagerStats,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) glibc: Option<GlibcMallinfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) jemalloc: Option<JemallocStats>,
}

#[derive(Debug, Serialize)]
pub(super) struct SessionCacheStats {
    pub(super) session_head_cache_entries: usize,
    pub(super) session_head_cache_keys: usize,
    pub(super) session_head_cache_bytes: usize,
    pub(super) session_head_cache_max_bytes: usize,
    pub(super) session_meta_cache_entries: usize,
    pub(super) session_meta_cache_bytes: usize,
    pub(super) session_event_heads: usize,
    pub(super) schedulers: usize,
    pub(super) broadcasters: usize,
    pub(super) broadcast_buffer_total: usize,
    pub(super) broadcast_buffer_max: usize,
    pub(super) broadcast_receivers_total: usize,
    pub(super) broadcast_receivers_max: usize,
    pub(super) running_sessions: usize,
    pub(super) active_task_refreshes: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct WorkspaceCacheStats {
    pub(super) file_completions_cache: usize,
    pub(super) file_completion_files: usize,
    pub(super) file_completion_bytes: usize,
    pub(super) workspace_file_completions_cache: usize,
    pub(super) workspace_file_completion_files: usize,
    pub(super) workspace_file_completion_bytes: usize,
    pub(super) git_status_snapshots: usize,
    pub(super) git_status_snapshot_bytes: usize,
    pub(super) git_status_watchers: usize,
    pub(super) workspace_active_snapshot_cache: usize,
    pub(super) workspace_active_snapshot_cache_bytes: usize,
    pub(super) workspace_active_snapshot_cache_max_bytes: usize,
    pub(super) workspace_active_heads_cache: usize,
    pub(super) workspace_active_heads_cache_bytes: usize,
    pub(super) workspace_active_heads_cache_max_bytes: usize,
    pub(super) worktree_bootstrap_gates: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct ProviderCacheStats {
    pub(super) adapters: usize,
    pub(super) statuses: usize,
    pub(super) options_cache: usize,
    pub(super) verify_cache: usize,
    pub(super) usage_cache: usize,
    pub(super) installs: usize,
}
