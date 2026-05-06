use std::sync::Arc;

use anyhow::Result;
use ctx_core::models::{
    Worktree, WorktreeVcsBaseResolution, WorktreeVcsComputeState, WorktreeVcsGitStatusSummary,
    WorktreeVcsSnapshot, WorktreeVcsSummary, WorktreeVcsTouchedFiles, WorktreeVcsTouchedFilesState,
};
use ctx_fs::vcs;
use ctx_workspace_services::worktree_vcs::{
    build_worktree_vcs_snapshot, WorktreeVcsSnapshotBuildParts, WorktreeVcsSnapshotCommitInfo,
};

use crate::api::sessions::{resolve_diff_base_with_meta, SessionDiffQuery};
use crate::daemon::AppState;
use crate::settings::ExecutionMode;
use crate::worktree_data_plane::resolve_worktree_data_plane;

use super::projection::publish_worktree_vcs_snapshot;
use super::sandbox::container_git_rev_parse;

#[allow(clippy::too_many_arguments)]
pub(super) async fn build_worktree_vcs_snapshot_from_parts(
    state: &Arc<AppState>,
    worktree: &Worktree,
    git_status: WorktreeVcsGitStatusSummary,
    touched_files: WorktreeVcsTouchedFiles,
    touched_files_state: WorktreeVcsTouchedFilesState,
    summary: WorktreeVcsSummary,
    compute_state: WorktreeVcsComputeState,
    resolution: Option<crate::api::sessions::WorktreeDiffBaseResolution>,
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
    let base_commit_sha = resolution.base_commit_sha.clone();
    let target_branch = resolution.target_branch.clone();
    let resolved_head_commit_sha = resolution.head_commit_sha.clone();
    let resolved_target_branch_commit_sha = resolution.target_branch_commit_sha.clone();
    let allow_live_target_lookup = unavailable_reason.is_none();
    let (head_commit_sha, target_branch_commit_sha) = if matches!(
        unavailable_reason,
        Some(ctx_core::models::DiffUnavailableReason::NoRepo)
    ) {
        (base_commit_sha.clone(), None)
    } else {
        let data_plane = resolve_worktree_data_plane(state, worktree).await?;
        let root = data_plane.live_worktree_root.as_path();
        if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
            let head = match resolved_head_commit_sha {
                Some(head) => head,
                None => container_git_rev_parse(state, worktree, "HEAD").await?,
            };
            let target = match (
                resolved_target_branch_commit_sha,
                target_branch.as_ref(),
                allow_live_target_lookup,
            ) {
                (Some(commit), _, _) => Some(commit),
                (None, Some(target_branch), true) => {
                    Some(container_git_rev_parse(state, worktree, target_branch).await?)
                }
                (None, _, _) => None,
            };
            (head, target)
        } else {
            let driver = vcs::driver_for_path(root).await?;
            let head = match resolved_head_commit_sha {
                Some(head) => head,
                None => driver.rev_parse_head(root).await?,
            };
            let target = match (
                resolved_target_branch_commit_sha,
                target_branch.as_ref(),
                allow_live_target_lookup,
            ) {
                (Some(commit), _, _) => Some(commit),
                (None, Some(target_branch), true) => {
                    Some(driver.rev_parse_ref(root, target_branch).await?)
                }
                (None, _, _) => None,
            };
            (head, target)
        }
    };
    let base_resolution = WorktreeVcsBaseResolution {
        kind: resolution.kind,
        target_source: resolution.target_source,
        error: resolution.error,
    };
    Ok(build_worktree_vcs_snapshot(WorktreeVcsSnapshotBuildParts {
        worktree_id: worktree.id,
        commit_info: WorktreeVcsSnapshotCommitInfo {
            base_commit_sha,
            head_commit_sha,
            target_branch,
            target_branch_commit_sha,
            base_resolution,
        },
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
    resolution: crate::api::sessions::WorktreeDiffBaseResolution,
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
    resolution: crate::api::sessions::WorktreeDiffBaseResolution,
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
