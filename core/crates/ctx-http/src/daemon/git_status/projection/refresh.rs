use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use ctx_core::models::{Worktree, WorktreeVcsBaseResolutionKind};
use ctx_workspace_services::worktree_vcs::{
    build_git_status_entries, build_git_status_summary, finish_worktree_vcs_refresh,
    is_no_vcs_repo_error, load_diff_file_count_from_source, load_diff_touched_entries_from_source,
    plan_worktree_vcs_summary_refresh, plan_worktree_vcs_touched_files_refresh,
    resolve_worktree_diff_base_from_source, worktree_vcs_projection_cache_state,
    worktree_vcs_summary_refresh_error_fallback, worktree_vcs_summary_refresh_from_file_count,
    worktree_vcs_summary_refresh_no_repo, worktree_vcs_touched_files_error_fallback,
    worktree_vcs_touched_files_from_entries, worktree_vcs_touched_files_large_change_set,
    worktree_vcs_touched_files_reuse, WorktreeDiffBaseResolution, WorktreeVcsDiffBaseQuery,
    WorktreeVcsSummaryRefreshPlan, WorktreeVcsTouchedFilesRefreshPlan,
};

use crate::daemon::AppState;

use super::super::snapshot::{
    build_worktree_vcs_snapshot_from_parts, publish_no_repo_snapshot, publish_unavailable_snapshot,
};
use super::super::source::HttpWorktreeVcsSource;
use super::super::worktree_has_vcs_repo;
use super::loading::load_git_status_snapshot;
use super::publish::publish_worktree_vcs_snapshot;

pub(in crate::daemon::git_status) async fn refresh_worktree_vcs_projection(
    state: &Arc<AppState>,
    worktree: &Worktree,
    refresh_summary: bool,
    refresh_touched_files: bool,
    force_emit: bool,
) -> Result<()> {
    if !state.worktree_vcs_enabled() {
        return Ok(());
    }
    let is_active = state.is_worktree_vcs_active(worktree.id).await;
    if !is_active {
        return Ok(());
    }
    let refresh_lock = state.worktree_vcs_refresh_lock(worktree.id).await;
    let _refresh_guard = refresh_lock.lock().await;

    let cached_snapshot = state.get_worktree_vcs_snapshot(worktree.id).await;
    let cached = worktree_vcs_projection_cache_state(cached_snapshot.as_ref());

    if !worktree_has_vcs_repo(state, worktree).await? {
        return publish_no_repo_snapshot(
            state,
            worktree,
            WorktreeDiffBaseResolution {
                base_commit_sha: worktree.base_commit_sha.clone(),
                head_commit_sha: None,
                target_branch_commit_sha: None,
                target_branch: None,
                target_source: None,
                kind: WorktreeVcsBaseResolutionKind::WorktreeBase,
                error: Some("worktree is not a vcs repository".to_string()),
                unavailable_reason: Some(ctx_core::models::DiffUnavailableReason::NoRepo),
                explicit_target: false,
            },
            force_emit,
        )
        .await;
    }

    let source = HttpWorktreeVcsSource::new(state, worktree);
    let resolution = resolve_worktree_diff_base_from_source(
        &source,
        worktree,
        WorktreeVcsDiffBaseQuery::default(),
    )
    .await;
    if let Some(reason) = resolution.unavailable_reason.clone() {
        return publish_unavailable_snapshot(state, worktree, resolution, force_emit, reason).await;
    }

    let summary_plan = plan_worktree_vcs_summary_refresh(&cached, refresh_summary);
    let (summary_result, summary_at) = match summary_plan {
        WorktreeVcsSummaryRefreshPlan::LoadFileCount => {
            match load_diff_file_count_from_source(&source, &resolution.base_commit_sha).await {
                Ok(file_count) => (
                    worktree_vcs_summary_refresh_from_file_count(file_count),
                    Some(Instant::now()),
                ),
                Err(err) if is_no_vcs_repo_error(&err) => {
                    (worktree_vcs_summary_refresh_no_repo(), None)
                }
                Err(err) => {
                    tracing::warn!(
                        worktree_id = %worktree.id.0,
                        "worktree diff file-count refresh failed: {err:#}"
                    );
                    (worktree_vcs_summary_refresh_error_fallback(&cached), None)
                }
            }
        }
        WorktreeVcsSummaryRefreshPlan::Reuse(result) => (result, None),
    };

    let touched_plan =
        plan_worktree_vcs_touched_files_refresh(&summary_result.summary, refresh_touched_files);
    let include_status_inventory = touched_plan.include_status_inventory();
    let git_snapshot = match load_git_status_snapshot(
        state,
        worktree,
        include_status_inventory,
        include_status_inventory,
    )
    .await
    {
        Ok(snapshot) => snapshot,
        Err(err) if is_no_vcs_repo_error(&err) => {
            return publish_no_repo_snapshot(state, worktree, resolution, force_emit).await;
        }
        Err(err) => return Err(err),
    };

    let git_status = build_git_status_summary(
        &git_snapshot,
        if include_status_inventory {
            build_git_status_entries(&git_snapshot.entries)
        } else {
            Vec::new()
        },
    );

    let touched_result = match touched_plan {
        WorktreeVcsTouchedFilesRefreshPlan::LargeChangeSet { file_count } => {
            worktree_vcs_touched_files_large_change_set(file_count)
        }
        WorktreeVcsTouchedFilesRefreshPlan::LoadDiff => {
            match load_diff_touched_entries_from_source(&source, &resolution.base_commit_sha).await
            {
                Ok(entries) => worktree_vcs_touched_files_from_entries(&entries),
                Err(err) if is_no_vcs_repo_error(&err) => {
                    return publish_no_repo_snapshot(state, worktree, resolution, force_emit).await;
                }
                Err(err) => {
                    tracing::warn!(
                        worktree_id = %worktree.id.0,
                        "worktree touched-file refresh failed: {err:#}"
                    );
                    worktree_vcs_touched_files_error_fallback(&cached)
                }
            }
        }
        WorktreeVcsTouchedFilesRefreshPlan::Reuse => worktree_vcs_touched_files_reuse(&cached),
    };

    let snapshot = build_worktree_vcs_snapshot_from_parts(
        state,
        worktree,
        git_status,
        touched_result.touched_files.clone(),
        touched_result.touched_files_state.clone(),
        summary_result.summary.clone(),
        summary_result.compute_state.clone(),
        Some(resolution),
        summary_result.available,
        summary_result.unavailable_reason,
    )
    .await?;

    publish_worktree_vcs_snapshot(state, worktree, snapshot, force_emit, summary_at).await;

    let mut runtime = state.workspaces.worktree_vcs_runtime.lock().await;
    let entry = runtime.entry(worktree.id).or_default();
    finish_worktree_vcs_refresh(
        entry,
        git_snapshot,
        touched_result.touched_files,
        touched_result.touched_files_state,
    );
    Ok(())
}
