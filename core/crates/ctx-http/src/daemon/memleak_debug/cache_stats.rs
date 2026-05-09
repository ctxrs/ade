use ctx_resource_utilization::memleak_debug::json_bytes;

use super::snapshot::{ProviderCacheStats, SessionCacheStats, WorkspaceCacheStats};
use crate::daemon::AppState;

pub(super) struct MemleakDebugCacheStats {
    pub(super) sessions: SessionCacheStats,
    pub(super) workspaces: WorkspaceCacheStats,
    pub(super) providers: ProviderCacheStats,
}

pub(super) async fn collect_memleak_debug_cache_stats(state: &AppState) -> MemleakDebugCacheStats {
    let sessions = collect_session_cache_stats(state).await;
    let workspaces = collect_workspace_cache_stats(state).await;
    let providers = collect_provider_cache_stats(state).await;
    MemleakDebugCacheStats {
        sessions,
        workspaces,
        providers,
    }
}

async fn collect_session_cache_stats(state: &AppState) -> SessionCacheStats {
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
}

async fn collect_workspace_cache_stats(state: &AppState) -> WorkspaceCacheStats {
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
}

async fn collect_provider_cache_stats(state: &AppState) -> ProviderCacheStats {
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
