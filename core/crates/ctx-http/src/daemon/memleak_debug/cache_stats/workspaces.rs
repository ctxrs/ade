use ctx_resource_utilization::memleak_debug::json_bytes;

use crate::daemon::AppState;

use super::WorkspaceCacheStats;

pub(super) async fn collect_workspace_cache_stats(state: &AppState) -> WorkspaceCacheStats {
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
