use std::sync::Arc;

use ctx_core::models::Worktree;
use ctx_settings_model::ExecutionMode;
use ctx_worktree_data_plane::resolve_worktree_data_plane_with_host as resolve_worktree_data_plane;
use ctx_worktree_vcs_service::{
    load_worktree_vcs_session_diff_from_host, load_worktree_vcs_session_diff_summary_from_host,
    WorktreeVcsDiffSummaryCounts,
};

use crate::daemon::DaemonState;

mod sandbox;
use sandbox::{container_diff_worktree, container_diff_worktree_summary};

pub async fn diff_worktree_for_session(
    state: &Arc<DaemonState>,
    worktree: &Worktree,
    base_commit_sha: &str,
) -> anyhow::Result<String> {
    let data_plane = resolve_worktree_data_plane(state.as_ref(), worktree).await?;
    if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
        return container_diff_worktree(state, worktree, base_commit_sha).await;
    }
    load_worktree_vcs_session_diff_from_host(&data_plane.live_worktree_root, base_commit_sha).await
}

pub async fn diff_worktree_summary_for_session(
    state: &Arc<DaemonState>,
    worktree: &Worktree,
    base_commit_sha: &str,
) -> anyhow::Result<WorktreeVcsDiffSummaryCounts> {
    let data_plane = resolve_worktree_data_plane(state.as_ref(), worktree).await?;
    if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
        return container_diff_worktree_summary(state, worktree, base_commit_sha).await;
    }
    load_worktree_vcs_session_diff_summary_from_host(
        &data_plane.live_worktree_root,
        base_commit_sha,
    )
    .await
}
