use std::sync::Arc;

use anyhow::Result;
use ctx_core::models::{
    Worktree, WorktreeVcsComputeState, WorktreeVcsGitStatusSummary, WorktreeVcsSnapshot,
    WorktreeVcsSummary, WorktreeVcsTouchedFiles, WorktreeVcsTouchedFilesState,
};
use ctx_workspace_services::worktree_vcs::{
    build_worktree_vcs_snapshot_from_source, WorktreeDiffBaseResolution,
};

use crate::daemon::DaemonState;

use super::projection::publish_worktree_vcs_snapshot;
use super::source::HttpWorktreeVcsSource;

#[allow(clippy::too_many_arguments)]
pub(super) async fn build_worktree_vcs_snapshot_from_parts(
    state: &Arc<DaemonState>,
    worktree: &Worktree,
    git_status: WorktreeVcsGitStatusSummary,
    touched_files: WorktreeVcsTouchedFiles,
    touched_files_state: WorktreeVcsTouchedFilesState,
    summary: WorktreeVcsSummary,
    compute_state: WorktreeVcsComputeState,
    resolution: Option<WorktreeDiffBaseResolution>,
    available: bool,
    unavailable_reason: Option<ctx_core::models::DiffUnavailableReason>,
) -> Result<WorktreeVcsSnapshot> {
    let source = HttpWorktreeVcsSource::new(state, worktree);
    build_worktree_vcs_snapshot_from_source(
        &source,
        worktree,
        git_status,
        touched_files,
        touched_files_state,
        summary,
        compute_state,
        resolution,
        available,
        unavailable_reason,
    )
    .await
}

pub(super) async fn publish_no_repo_snapshot(
    state: &Arc<DaemonState>,
    worktree: &Worktree,
    resolution: WorktreeDiffBaseResolution,
    force_emit: bool,
) -> Result<()> {
    publish_unavailable_snapshot(
        state,
        worktree,
        resolution,
        force_emit,
        ctx_core::models::DiffUnavailableReason::NoRepo,
    )
    .await
}

pub(super) async fn publish_unavailable_snapshot(
    state: &Arc<DaemonState>,
    worktree: &Worktree,
    resolution: WorktreeDiffBaseResolution,
    force_emit: bool,
    reason: ctx_core::models::DiffUnavailableReason,
) -> Result<()> {
    let snapshot = build_worktree_vcs_snapshot_from_parts(
        state,
        worktree,
        WorktreeVcsGitStatusSummary::default(),
        WorktreeVcsTouchedFiles::default(),
        WorktreeVcsTouchedFilesState::NotLoaded,
        WorktreeVcsSummary::default(),
        WorktreeVcsComputeState::Ready,
        Some(resolution),
        false,
        Some(reason),
    )
    .await?;
    publish_worktree_vcs_snapshot(state, worktree, snapshot, force_emit, None).await;
    Ok(())
}
