use std::sync::Arc;

use anyhow::Result;
use ctx_core::models::{
    Worktree, WorktreeVcsComputeState, WorktreeVcsGitStatusSummary, WorktreeVcsSnapshot,
    WorktreeVcsSummary, WorktreeVcsTouchedFiles, WorktreeVcsTouchedFilesState,
};
use ctx_workspace_services::worktree_vcs::{
    build_worktree_vcs_snapshot, plan_worktree_vcs_commit_info,
    resolve_worktree_diff_base_from_source, resolve_worktree_vcs_commit_lookup_from_source,
    WorktreeDiffBaseResolution, WorktreeVcsDiffBaseQuery, WorktreeVcsSnapshotBuildParts,
};

use crate::daemon::AppState;

use super::projection::publish_worktree_vcs_snapshot;
use super::source::HttpWorktreeVcsSource;

#[allow(clippy::too_many_arguments)]
pub(super) async fn build_worktree_vcs_snapshot_from_parts(
    state: &Arc<AppState>,
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
    let resolution = match resolution {
        Some(resolution) => resolution,
        None => {
            let source = HttpWorktreeVcsSource::new(state, worktree);
            resolve_worktree_diff_base_from_source(
                &source,
                worktree,
                WorktreeVcsDiffBaseQuery::default(),
            )
            .await
        }
    };
    let commit_plan = plan_worktree_vcs_commit_info(resolution, unavailable_reason.clone());
    let source = HttpWorktreeVcsSource::new(state, worktree);
    let head_commit_sha =
        resolve_worktree_vcs_commit_lookup_from_source(&source, &commit_plan.head_commit_sha)
            .await?
            .ok_or_else(|| anyhow::anyhow!("worktree vcs head commit lookup was missing"))?;
    let target_branch_commit_sha = resolve_worktree_vcs_commit_lookup_from_source(
        &source,
        &commit_plan.target_branch_commit_sha,
    )
    .await?;
    let commit_info = commit_plan.into_commit_info(head_commit_sha, target_branch_commit_sha);
    Ok(build_worktree_vcs_snapshot(WorktreeVcsSnapshotBuildParts {
        worktree_id: worktree.id,
        commit_info,
        compute_state,
        summary,
        git_status,
        touched_files,
        touched_files_state,
        available,
        unavailable_reason,
    }))
}

pub(super) async fn publish_no_repo_snapshot(
    state: &Arc<AppState>,
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
    state: &Arc<AppState>,
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
