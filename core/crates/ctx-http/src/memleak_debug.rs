use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Utc};
use ctx_resource_utilization::memleak_debug::{
    append_memleak_debug_log, json_bytes, read_memleak_debug_process_stats, GlibcMallinfo,
    JemallocStats, MemleakDebugConfig,
};
use serde::Serialize;
use tokio::time::MissedTickBehavior;

use crate::daemon::AppState;
use crate::terminals::TerminalManagerStats;
use crate::web_sessions::WebSessionManagerStats;
use ctx_harness_runtime::HarnessRuntimeStats;
use ctx_observability::logs;
use ctx_observability::perf_telemetry::PerfTelemetryStats;
use ctx_store::StoreManagerStats;
use ctx_workspace_active_snapshot::WorkspaceActiveSnapshotStats;

#[derive(Debug, Serialize)]
struct MemleakDebugSnapshot {
    occurred_at: DateTime<Utc>,
    rss_bytes: u64,
    thread_count: u32,
    sessions: SessionCacheStats,
    workspaces: WorkspaceCacheStats,
    providers: ProviderCacheStats,
    active_snapshot: WorkspaceActiveSnapshotStats,
    terminals: TerminalManagerStats,
    perf_telemetry: PerfTelemetryStats,
    web_sessions: WebSessionManagerStats,
    harness_runtime: HarnessRuntimeStats,
    stores: StoreManagerStats,
    #[serde(skip_serializing_if = "Option::is_none")]
    glibc: Option<GlibcMallinfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    jemalloc: Option<JemallocStats>,
}

#[derive(Debug, Serialize)]
struct SessionCacheStats {
    session_head_cache_entries: usize,
    session_head_cache_keys: usize,
    session_head_cache_bytes: usize,
    session_head_cache_max_bytes: usize,
    session_meta_cache_entries: usize,
    session_meta_cache_bytes: usize,
    session_event_heads: usize,
    schedulers: usize,
    broadcasters: usize,
    broadcast_buffer_total: usize,
    broadcast_buffer_max: usize,
    broadcast_receivers_total: usize,
    broadcast_receivers_max: usize,
    running_sessions: usize,
    active_task_refreshes: usize,
}

#[derive(Debug, Serialize)]
struct WorkspaceCacheStats {
    file_completions_cache: usize,
    file_completion_files: usize,
    file_completion_bytes: usize,
    workspace_file_completions_cache: usize,
    workspace_file_completion_files: usize,
    workspace_file_completion_bytes: usize,
    git_status_snapshots: usize,
    git_status_snapshot_bytes: usize,
    git_status_watchers: usize,
    workspace_active_snapshot_cache: usize,
    workspace_active_snapshot_cache_bytes: usize,
    workspace_active_snapshot_cache_max_bytes: usize,
    workspace_active_heads_cache: usize,
    workspace_active_heads_cache_bytes: usize,
    workspace_active_heads_cache_max_bytes: usize,
    worktree_bootstrap_gates: usize,
}

#[derive(Debug, Serialize)]
struct ProviderCacheStats {
    adapters: usize,
    statuses: usize,
    options_cache: usize,
    verify_cache: usize,
    usage_cache: usize,
    installs: usize,
}

pub fn spawn_memleak_debug(state: Arc<AppState>) {
    let config = MemleakDebugConfig::from_env();
    if !config.enabled {
        return;
    }
    let mut shutdown_rx = state.core.shutdown_tx.subscribe();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(config.interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => break,
                _ = ticker.tick() => {
                    if let Err(err) = sample_once(&state).await {
                        tracing::warn!("memleak debug sample failed: {err:#}");
                    }
                }
            }
        }
    });
}

async fn sample_once(state: &Arc<AppState>) -> Result<()> {
    let sessions = {
        let head_cache = state.sessions.session_head_cache.lock().await;
        let head_cache_entries = head_cache.len();
        let head_cache_keys = head_cache.values().map(|entry| entry.value.len()).sum();
        let mut session_head_cache_bytes = 0;
        let mut session_head_cache_max_bytes = 0;
        for entry in head_cache.values() {
            for head in entry.value.values() {
                let bytes = json_bytes(head);
                session_head_cache_bytes += bytes;
                if bytes > session_head_cache_max_bytes {
                    session_head_cache_max_bytes = bytes;
                }
            }
        }
        drop(head_cache);

        let session_meta_cache = state.sessions.session_meta_cache.lock().await;
        let session_meta_cache_entries = session_meta_cache.len();
        let mut session_meta_cache_bytes = 0;
        for entry in session_meta_cache.values() {
            session_meta_cache_bytes += json_bytes(&entry.value);
        }
        drop(session_meta_cache);

        let session_event_heads = state.sessions.session_event_heads.lock().await.len();
        let schedulers = state.sessions.schedulers.lock().await.len();
        let broadcasters_guard = state.sessions.broadcasters.lock().await;
        let broadcasters = broadcasters_guard.len();
        let mut broadcast_buffer_total = 0;
        let mut broadcast_buffer_max = 0;
        let mut broadcast_receivers_total = 0;
        let mut broadcast_receivers_max = 0;
        for entry in broadcasters_guard.values() {
            let sender = &entry.value;
            let len = sender.len();
            broadcast_buffer_total += len;
            if len > broadcast_buffer_max {
                broadcast_buffer_max = len;
            }
            let receivers = sender.receiver_count();
            broadcast_receivers_total += receivers;
            if receivers > broadcast_receivers_max {
                broadcast_receivers_max = receivers;
            }
        }
        drop(broadcasters_guard);
        let running_sessions = state.sessions.running_sessions.lock().await.len();
        let active_task_refreshes = state.sessions.active_task_refreshes.lock().await.len();

        SessionCacheStats {
            session_head_cache_entries: head_cache_entries,
            session_head_cache_keys: head_cache_keys,
            session_head_cache_bytes,
            session_head_cache_max_bytes,
            session_meta_cache_entries,
            session_meta_cache_bytes,
            session_event_heads,
            schedulers,
            broadcasters,
            broadcast_buffer_total,
            broadcast_buffer_max,
            broadcast_receivers_total,
            broadcast_receivers_max,
            running_sessions,
            active_task_refreshes,
        }
    };

    let workspaces = {
        let file_completions_guard = state.workspaces.file_completions_cache.lock().await;
        let file_completions_cache = file_completions_guard.len();
        let mut file_completion_files = 0;
        let mut file_completion_bytes = 0;
        for entry in file_completions_guard.values() {
            let files = &entry.value.files;
            file_completion_files += files.len();
            file_completion_bytes += files.iter().map(|path| path.len()).sum::<usize>();
        }
        drop(file_completions_guard);

        let workspace_file_completions_guard = state
            .workspaces
            .workspace_file_completions_cache
            .lock()
            .await;
        let workspace_file_completions_cache = workspace_file_completions_guard.len();
        let mut workspace_file_completion_files = 0;
        let mut workspace_file_completion_bytes = 0;
        for entry in workspace_file_completions_guard.values() {
            let files = &entry.value.files;
            workspace_file_completion_files += files.len();
            workspace_file_completion_bytes += files.iter().map(|path| path.len()).sum::<usize>();
        }
        drop(workspace_file_completions_guard);

        let git_status_guard = state.workspaces.git_status_snapshots.lock().await;
        let git_status_snapshots = git_status_guard.len();
        let mut git_status_snapshot_bytes = 0;
        for entry in git_status_guard.values() {
            git_status_snapshot_bytes += entry.value.payload.len();
        }
        drop(git_status_guard);
        let git_status_watchers = state.workspaces.git_status_watchers.lock().await.len();
        let workspace_snapshot_guard = state
            .workspaces
            .workspace_active_snapshot_cache
            .lock()
            .await;
        let workspace_active_snapshot_cache = workspace_snapshot_guard.len();
        let mut workspace_active_snapshot_cache_bytes = 0;
        let mut workspace_active_snapshot_cache_max_bytes = 0;
        for entry in workspace_snapshot_guard.values() {
            let bytes = json_bytes(&entry.value.snapshot);
            workspace_active_snapshot_cache_bytes += bytes;
            if bytes > workspace_active_snapshot_cache_max_bytes {
                workspace_active_snapshot_cache_max_bytes = bytes;
            }
        }
        drop(workspace_snapshot_guard);

        let workspace_heads_guard = state.workspaces.workspace_active_heads_cache.lock().await;
        let workspace_active_heads_cache = workspace_heads_guard.len();
        let mut workspace_active_heads_cache_bytes = 0;
        let mut workspace_active_heads_cache_max_bytes = 0;
        for entry in workspace_heads_guard.values() {
            let bytes = json_bytes(&entry.value.batch);
            workspace_active_heads_cache_bytes += bytes;
            if bytes > workspace_active_heads_cache_max_bytes {
                workspace_active_heads_cache_max_bytes = bytes;
            }
        }
        drop(workspace_heads_guard);
        let worktree_bootstrap_gates = state.workspaces.worktree_bootstrap_gates.lock().await.len();

        WorkspaceCacheStats {
            file_completions_cache,
            file_completion_files,
            file_completion_bytes,
            workspace_file_completions_cache,
            workspace_file_completion_files,
            workspace_file_completion_bytes,
            git_status_snapshots,
            git_status_snapshot_bytes,
            git_status_watchers,
            workspace_active_snapshot_cache,
            workspace_active_snapshot_cache_bytes,
            workspace_active_snapshot_cache_max_bytes,
            workspace_active_heads_cache,
            workspace_active_heads_cache_bytes,
            workspace_active_heads_cache_max_bytes,
            worktree_bootstrap_gates,
        }
    };

    let providers = {
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
    };

    let active_snapshot = state.workspaces.workspace_active_snapshot.stats().await;
    let terminals = state.transport.terminals.stats().await;
    let perf_telemetry = state.telemetry.perf_telemetry.stats();
    let web_sessions = state.transport.web_sessions.stats().await;
    let harness_runtime = state.execution.harness.stats().await;
    let stores = state.core.stores.stats().await;
    let process = read_memleak_debug_process_stats();

    let snapshot = MemleakDebugSnapshot {
        occurred_at: Utc::now(),
        rss_bytes: process.rss_bytes,
        thread_count: process.thread_count,
        sessions,
        workspaces,
        providers,
        active_snapshot,
        terminals,
        perf_telemetry,
        web_sessions,
        harness_runtime,
        stores,
        glibc: process.glibc,
        jemalloc: process.jemalloc,
    };

    let logs_dir = logs::logs_dir(&state.core.data_root);
    append_memleak_debug_log(&logs_dir, &snapshot).await?;
    Ok(())
}
