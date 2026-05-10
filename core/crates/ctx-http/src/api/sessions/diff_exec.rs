use super::*;
use ctx_settings_model::ExecutionMode;
use ctx_workspace_services::worktree_vcs::WorktreeVcsDiffSummaryCounts;
use ctx_worktree_data_plane::resolve_worktree_data_plane_with_host as resolve_worktree_data_plane;

mod sandbox;
use sandbox::{container_diff_worktree, container_diff_worktree_summary};

pub(crate) async fn diff_worktree_for_session(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
) -> anyhow::Result<String> {
    let data_plane = resolve_worktree_data_plane(state.as_ref(), worktree).await?;
    if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
        return container_diff_worktree(state, worktree, base_commit_sha).await;
    }
    ctx_fs::worktrees::diff_worktree(
        data_plane.live_worktree_root.to_string_lossy().as_ref(),
        base_commit_sha,
    )
    .await
}

pub(crate) async fn diff_worktree_summary_for_session(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
) -> anyhow::Result<WorktreeVcsDiffSummaryCounts> {
    let data_plane = resolve_worktree_data_plane(state.as_ref(), worktree).await?;
    if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
        return container_diff_worktree_summary(state, worktree, base_commit_sha).await;
    }
    let (file_count, line_additions, line_deletions) = ctx_fs::worktrees::diff_worktree_summary(
        data_plane.live_worktree_root.to_string_lossy().as_ref(),
        base_commit_sha,
    )
    .await?;
    Ok(WorktreeVcsDiffSummaryCounts {
        file_count,
        line_additions,
        line_deletions,
    })
}
