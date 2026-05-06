use std::sync::Arc;

use anyhow::Result;
use ctx_core::models::{
    Worktree, WorktreeVcsComputeState, WorktreeVcsGitStatusSummary, WorktreeVcsSnapshot,
    WorktreeVcsSummary, WorktreeVcsTouchedFiles, WorktreeVcsTouchedFilesState,
};
use ctx_fs::vcs;
use ctx_workspace_services::worktree_vcs::{
    build_worktree_vcs_snapshot, plan_worktree_vcs_commit_info, WorktreeDiffBaseResolution,
    WorktreeVcsCommitLookup, WorktreeVcsSnapshotBuildParts,
};

use crate::api::sessions::{resolve_diff_base_with_meta, SessionDiffQuery};
use crate::daemon::AppState;
use crate::settings::ExecutionMode;
use crate::worktree_data_plane::resolve_worktree_data_plane;

use super::projection::publish_worktree_vcs_snapshot;
use super::sandbox::container_git_rev_parse;

async fn resolve_worktree_vcs_commit_lookup(
    state: &Arc<AppState>,
    worktree: &Worktree,
    lookup: &WorktreeVcsCommitLookup,
) -> Result<Option<String>> {
    match lookup {
        WorktreeVcsCommitLookup::Resolved(commit) => Ok(Some(commit.clone())),
        WorktreeVcsCommitLookup::Missing => Ok(None),
        WorktreeVcsCommitLookup::Head => {
            let data_plane = resolve_worktree_data_plane(state, worktree).await?;
            let root = data_plane.live_worktree_root.as_path();
            if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
                Ok(Some(
                    container_git_rev_parse(state, worktree, "HEAD").await?,
                ))
            } else {
                let driver = vcs::driver_for_path(root).await?;
                Ok(Some(driver.rev_parse_head(root).await?))
            }
        }
        WorktreeVcsCommitLookup::TargetBranch(target_branch) => {
            let data_plane = resolve_worktree_data_plane(state, worktree).await?;
            let root = data_plane.live_worktree_root.as_path();
            if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
                Ok(Some(
                    container_git_rev_parse(state, worktree, target_branch).await?,
                ))
            } else {
                let driver = vcs::driver_for_path(root).await?;
                Ok(Some(driver.rev_parse_ref(root, target_branch).await?))
            }
        }
    }
}

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
            let data_plane = resolve_worktree_data_plane(state, worktree).await?;
            let store = state.store_for_worktree(worktree.id).await?;
            resolve_diff_base_with_meta(
                state,
                &store,
                &data_plane.workspace,
                worktree,
                &SessionDiffQuery::default(),
            )
            .await
        }
    };
    let commit_plan = plan_worktree_vcs_commit_info(resolution, unavailable_reason.clone());
    let head_commit_sha =
        resolve_worktree_vcs_commit_lookup(state, worktree, &commit_plan.head_commit_sha)
            .await?
            .ok_or_else(|| anyhow::anyhow!("worktree vcs head commit lookup was missing"))?;
    let target_branch_commit_sha =
        resolve_worktree_vcs_commit_lookup(state, worktree, &commit_plan.target_branch_commit_sha)
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
