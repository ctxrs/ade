use std::collections::HashSet;
use std::sync::Arc;

use ctx_core::ids::WorktreeId;
use ctx_resource_utilization::memleak_debug::json_bytes;
use serde::Serialize;
use tokio::sync::Mutex;

use crate::daemon::state::{
    WorkspaceActiveHeadsCache, WorkspaceActiveSnapshotCache, WorkspaceFileCompletionsCache,
    WorkspaceRuntime, WorktreeBootstrapGateCache, WorktreeFileCompletionsCache,
    WorktreeGitStatusSnapshotCache,
};

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct WorkspaceCacheDebugStats {
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

#[derive(Clone)]
pub(in crate::daemon) struct WorkspaceCacheDebugStatsHost {
    file_completions_cache: WorktreeFileCompletionsCache,
    workspace_file_completions_cache: WorkspaceFileCompletionsCache,
    git_status_snapshots: WorktreeGitStatusSnapshotCache,
    git_status_watchers: Arc<Mutex<HashSet<WorktreeId>>>,
    workspace_active_snapshot_cache: WorkspaceActiveSnapshotCache,
    workspace_active_heads_cache: WorkspaceActiveHeadsCache,
    worktree_bootstrap_gates: WorktreeBootstrapGateCache,
}

pub(in crate::daemon) struct WorkspaceCacheDebugStatsHostParts {
    pub(in crate::daemon) file_completions_cache: WorktreeFileCompletionsCache,
    pub(in crate::daemon) workspace_file_completions_cache: WorkspaceFileCompletionsCache,
    pub(in crate::daemon) git_status_snapshots: WorktreeGitStatusSnapshotCache,
    pub(in crate::daemon) git_status_watchers: Arc<Mutex<HashSet<WorktreeId>>>,
    pub(in crate::daemon) workspace_active_snapshot_cache: WorkspaceActiveSnapshotCache,
    pub(in crate::daemon) workspace_active_heads_cache: WorkspaceActiveHeadsCache,
    pub(in crate::daemon) worktree_bootstrap_gates: WorktreeBootstrapGateCache,
}

impl WorkspaceCacheDebugStatsHost {
    pub(in crate::daemon) fn new(parts: WorkspaceCacheDebugStatsHostParts) -> Self {
        Self {
            file_completions_cache: parts.file_completions_cache,
            workspace_file_completions_cache: parts.workspace_file_completions_cache,
            git_status_snapshots: parts.git_status_snapshots,
            git_status_watchers: parts.git_status_watchers,
            workspace_active_snapshot_cache: parts.workspace_active_snapshot_cache,
            workspace_active_heads_cache: parts.workspace_active_heads_cache,
            worktree_bootstrap_gates: parts.worktree_bootstrap_gates,
        }
    }

    pub(in crate::daemon) async fn cache_debug_stats(&self) -> WorkspaceCacheDebugStats {
        collect_workspace_cache_debug_stats(self).await
    }
}

pub(in crate::daemon) fn workspace_cache_debug_stats_host_from_runtime(
    runtime: &WorkspaceRuntime,
) -> WorkspaceCacheDebugStatsHost {
    WorkspaceCacheDebugStatsHost::new(WorkspaceCacheDebugStatsHostParts {
        file_completions_cache: Arc::clone(&runtime.file_completions_cache),
        workspace_file_completions_cache: Arc::clone(&runtime.workspace_file_completions_cache),
        git_status_snapshots: Arc::clone(&runtime.git_status_snapshots),
        git_status_watchers: Arc::clone(&runtime.git_status_watchers),
        workspace_active_snapshot_cache: Arc::clone(&runtime.workspace_active_snapshot_cache),
        workspace_active_heads_cache: Arc::clone(&runtime.workspace_active_heads_cache),
        worktree_bootstrap_gates: Arc::clone(&runtime.worktree_bootstrap_gates),
    })
}

async fn collect_workspace_cache_debug_stats(
    host: &WorkspaceCacheDebugStatsHost,
) -> WorkspaceCacheDebugStats {
    let file_completions_guard = host.file_completions_cache.lock().await;
    let file_completions_cache = file_completions_guard.len();
    let mut file_completion_files = 0;
    let mut file_completion_bytes = 0;
    for entry in file_completions_guard.values() {
        let files = &entry.value.files;
        file_completion_files += files.len();
        file_completion_bytes += files.iter().map(|path| path.len()).sum::<usize>();
    }
    drop(file_completions_guard);

    let workspace_file_completions_guard = host.workspace_file_completions_cache.lock().await;
    let workspace_file_completions_cache = workspace_file_completions_guard.len();
    let mut workspace_file_completion_files = 0;
    let mut workspace_file_completion_bytes = 0;
    for entry in workspace_file_completions_guard.values() {
        let files = &entry.value.files;
        workspace_file_completion_files += files.len();
        workspace_file_completion_bytes += files.iter().map(|path| path.len()).sum::<usize>();
    }
    drop(workspace_file_completions_guard);

    let git_status_guard = host.git_status_snapshots.lock().await;
    let git_status_snapshots = git_status_guard.len();
    let mut git_status_snapshot_bytes = 0;
    for entry in git_status_guard.values() {
        git_status_snapshot_bytes += entry.value.payload.len();
    }
    drop(git_status_guard);
    let git_status_watchers = host.git_status_watchers.lock().await.len();

    let workspace_snapshot_guard = host.workspace_active_snapshot_cache.lock().await;
    let workspace_active_snapshot_cache = workspace_snapshot_guard.len();
    let mut workspace_active_snapshot_cache_bytes = 0;
    let mut workspace_active_snapshot_cache_max_bytes = 0;
    for entry in workspace_snapshot_guard.values() {
        let bytes = json_bytes(&entry.value.snapshot);
        workspace_active_snapshot_cache_bytes += bytes;
        workspace_active_snapshot_cache_max_bytes =
            workspace_active_snapshot_cache_max_bytes.max(bytes);
    }
    drop(workspace_snapshot_guard);

    let workspace_heads_guard = host.workspace_active_heads_cache.lock().await;
    let workspace_active_heads_cache = workspace_heads_guard.len();
    let mut workspace_active_heads_cache_bytes = 0;
    let mut workspace_active_heads_cache_max_bytes = 0;
    for entry in workspace_heads_guard.values() {
        let bytes = json_bytes(&entry.value.batch);
        workspace_active_heads_cache_bytes += bytes;
        workspace_active_heads_cache_max_bytes = workspace_active_heads_cache_max_bytes.max(bytes);
    }
    drop(workspace_heads_guard);
    let worktree_bootstrap_gates = host.worktree_bootstrap_gates.lock().await.len();

    WorkspaceCacheDebugStats {
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
