use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use ctx_core::models::{
    Worktree, WorktreeVcsBaseResolution, WorktreeVcsComputeState, WorktreeVcsFreshness,
    WorktreeVcsGitStatusSummary, WorktreeVcsSnapshot, WorktreeVcsSummary, WorktreeVcsTouchedFile,
    WorktreeVcsTouchedFiles, WorktreeVcsTouchedFilesState,
};
use ctx_fs::vcs;

use crate::api::sessions::{resolve_diff_base_with_meta, SessionDiffQuery};
use crate::daemon::AppState;
use crate::settings::ExecutionMode;
use crate::worktree_data_plane::resolve_worktree_data_plane;

use super::model::{GitStatusEntry, GitStatusSnapshot};
use super::projection::publish_worktree_vcs_snapshot;
use super::sandbox::container_git_rev_parse;

pub(super) const WORKTREE_VCS_TOUCHED_FILES_CAP: usize = 200;
// Above this count, the product surfaces an exact summary but does not compute
// or stream file-by-file review inventory.
pub(super) const WORKTREE_VCS_REVIEWABLE_FILE_LIMIT: i64 = 300;
pub(super) const WORKTREE_VCS_SNAPSHOT_SCHEMA_VERSION: i64 = 2;

pub(super) fn now_epoch_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as i64)
        .unwrap_or(0)
}

pub(super) fn snapshot_fingerprint(snapshot: &WorktreeVcsSnapshot) -> String {
    let mut copy = snapshot.clone();
    copy.rev = 0;
    copy.emitted_at_ms = 0;
    serde_json::to_string(&copy).unwrap_or_default()
}

pub(super) fn build_touched_files(entries: &[WorktreeVcsTouchedFile]) -> WorktreeVcsTouchedFiles {
    let total_count = entries.len() as i64;
    let truncated = entries.len() > WORKTREE_VCS_TOUCHED_FILES_CAP;
    let mut items = Vec::new();
    for entry in entries.iter().take(WORKTREE_VCS_TOUCHED_FILES_CAP) {
        items.push(entry.clone());
    }
    WorktreeVcsTouchedFiles {
        items,
        truncated,
        total_count: Some(total_count),
    }
}

pub(super) fn build_large_change_set_touched_files(file_count: i64) -> WorktreeVcsTouchedFiles {
    WorktreeVcsTouchedFiles {
        items: Vec::new(),
        truncated: true,
        total_count: Some(file_count),
    }
}

pub(super) fn build_git_status_entries(entries: &[GitStatusEntry]) -> Vec<WorktreeVcsTouchedFile> {
    let mut out = Vec::new();
    for entry in entries.iter().take(WORKTREE_VCS_TOUCHED_FILES_CAP) {
        out.push(WorktreeVcsTouchedFile {
            path: entry.path.clone(),
            orig_path: entry.orig_path.clone(),
            index_status: Some(entry.index_status.clone()),
            worktree_status: Some(entry.worktree_status.clone()),
        });
    }
    out
}

pub(super) fn build_git_status_summary(
    snapshot: &GitStatusSnapshot,
    entries: Vec<WorktreeVcsTouchedFile>,
) -> WorktreeVcsGitStatusSummary {
    WorktreeVcsGitStatusSummary {
        raw: String::new(),
        summary_line: snapshot.summary_line.clone(),
        branch: snapshot.branch.clone(),
        upstream: snapshot.upstream.clone(),
        ahead: snapshot.ahead,
        behind: snapshot.behind,
        detached: snapshot.detached,
        staged: snapshot.staged,
        unstaged: snapshot.unstaged,
        untracked: snapshot.untracked,
        entries,
    }
}

pub(super) fn summary_from_file_count(file_count: i64) -> WorktreeVcsSummary {
    WorktreeVcsSummary {
        file_count: Some(file_count),
        line_additions: None,
        line_deletions: None,
        line_count: None,
    }
}

pub(super) fn snapshot_for_durable_cache(snapshot: &WorktreeVcsSnapshot) -> WorktreeVcsSnapshot {
    let mut durable = snapshot.clone();
    durable.touched_files = WorktreeVcsTouchedFiles::default();
    durable.touched_files_state = WorktreeVcsTouchedFilesState::NotLoaded;
    durable.git_status.raw.clear();
    durable.git_status.entries.clear();
    durable
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
    let freshness = derive_worktree_vcs_freshness(&compute_state, &summary);
    Ok(WorktreeVcsSnapshot {
        worktree_id: worktree.id,
        rev: 0,
        emitted_at_ms: 0,
        base_commit_sha,
        head_commit_sha,
        target_branch,
        target_branch_commit_sha,
        base_resolution,
        compute_state,
        summary,
        git_status,
        touched_files,
        touched_files_state,
        freshness,
        available,
        unavailable_reason,
        schema_version: WORKTREE_VCS_SNAPSHOT_SCHEMA_VERSION,
    })
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

pub(super) fn summary_has_counts(summary: &WorktreeVcsSummary) -> bool {
    summary.file_count.is_some()
        || summary.line_additions.is_some()
        || summary.line_deletions.is_some()
        || summary.line_count.is_some()
}

pub(super) fn derive_worktree_vcs_freshness(
    compute_state: &WorktreeVcsComputeState,
    summary: &WorktreeVcsSummary,
) -> WorktreeVcsFreshness {
    match compute_state {
        WorktreeVcsComputeState::Ready => WorktreeVcsFreshness::Fresh,
        WorktreeVcsComputeState::Error => WorktreeVcsFreshness::Error,
        WorktreeVcsComputeState::Computing => {
            if summary_has_counts(summary) {
                WorktreeVcsFreshness::Stale
            } else {
                WorktreeVcsFreshness::Refreshing
            }
        }
    }
}
