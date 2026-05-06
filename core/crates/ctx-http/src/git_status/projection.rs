use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use ctx_core::models::{
    Worktree, WorktreeVcsBaseResolutionKind, WorktreeVcsComputeState, WorktreeVcsSnapshot,
    WorktreeVcsSummary, WorktreeVcsTouchedFilesState,
};
use ctx_workspace_services::worktree_vcs::{
    build_git_status_entries, build_git_status_summary, build_large_change_set_touched_files,
    build_touched_files, snapshot_for_durable_cache, summary_from_file_count, summary_has_counts,
    GitStatusEntry, GitStatusSnapshot, WORKTREE_VCS_REVIEWABLE_FILE_LIMIT,
    WORKTREE_VCS_SNAPSHOT_SCHEMA_VERSION,
};

use crate::api::sessions::{resolve_diff_base_with_meta, SessionDiffQuery};
use crate::daemon::AppState;
use crate::settings::ExecutionMode;
use crate::worktree_data_plane::resolve_worktree_data_plane;

use super::diff_paths::{load_diff_file_count, load_diff_touched_entries};
use super::sandbox::container_git_status_structured;
use super::snapshot::{
    build_worktree_vcs_snapshot_from_parts, now_epoch_ms, publish_no_repo_snapshot,
    publish_unavailable_snapshot, snapshot_fingerprint,
};
use super::{
    vcs_driver_for_worktree, worktree_has_vcs_repo, GIT_STATUS_DEBOUNCE_MS,
    GIT_STATUS_MAX_INTERVAL_MS,
};

pub async fn load_git_status_snapshot(
    state: &Arc<AppState>,
    worktree: &Worktree,
    include_untracked_files: bool,
    include_entries: bool,
) -> Result<GitStatusSnapshot> {
    let data_plane = resolve_worktree_data_plane(state, worktree).await?;
    let root = data_plane.live_worktree_root.as_path();
    let structured = if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
        container_git_status_structured(state, worktree, include_untracked_files, include_entries)
            .await?
    } else {
        let vcs = vcs_driver_for_worktree(worktree);
        vcs.status_structured(root, include_untracked_files, include_entries)
            .await?
    };
    Ok(GitStatusSnapshot {
        raw: structured.raw,
        summary_line: structured.branch.summary_line,
        branch: structured.branch.branch,
        upstream: structured.branch.upstream,
        ahead: structured.branch.ahead,
        behind: structured.branch.behind,
        detached: structured.branch.detached,
        staged: structured.staged,
        unstaged: structured.unstaged,
        untracked: structured.untracked,
        entries: if include_entries {
            structured
                .entries
                .into_iter()
                .map(|entry| GitStatusEntry {
                    path: entry.path,
                    orig_path: entry.orig_path,
                    index_status: entry.index_status,
                    worktree_status: entry.worktree_status,
                })
                .collect()
        } else {
            Vec::new()
        },
        entries_total_count: structured.total_count,
        entries_truncated: structured.truncated,
    })
}

pub(super) async fn persist_worktree_vcs_snapshot(
    state: &Arc<AppState>,
    worktree: &Worktree,
    snapshot: &WorktreeVcsSnapshot,
) {
    let durable = snapshot_for_durable_cache(snapshot);
    let Ok(store) = state.store_for_worktree(worktree.id).await else {
        return;
    };
    if let Err(err) = store
        .upsert_worktree_vcs_snapshot_cache(worktree, &durable)
        .await
    {
        tracing::warn!(
            worktree_id = %worktree.id.0,
            "persisting worktree vcs snapshot cache failed: {err:#}"
        );
    }
}

pub(super) async fn publish_worktree_vcs_snapshot(
    state: &Arc<AppState>,
    worktree: &Worktree,
    snapshot: WorktreeVcsSnapshot,
    force_emit: bool,
    summary_at: Option<Instant>,
) -> Option<WorktreeVcsSnapshot> {
    let published = upsert_worktree_vcs_snapshot(state, snapshot, force_emit, summary_at).await?;
    persist_worktree_vcs_snapshot(state, worktree, &published).await;
    if state.is_worktree_vcs_active(worktree.id).await {
        state
            .workspaces
            .workspace_active_snapshot
            .publish_worktree_vcs_snapshot(worktree.workspace_id, published.clone())
            .await;
    }
    Some(published)
}

pub(super) async fn upsert_worktree_vcs_snapshot(
    state: &Arc<AppState>,
    mut snapshot: WorktreeVcsSnapshot,
    force_emit: bool,
    summary_at: Option<Instant>,
) -> Option<WorktreeVcsSnapshot> {
    let now = Instant::now();
    let fingerprint = snapshot_fingerprint(&snapshot);
    let active = state.workspaces.worktree_vcs_active.lock().await;
    if active.get(&snapshot.worktree_id).copied().unwrap_or(0) == 0 {
        return None;
    }
    let mut cache = state.workspaces.worktree_vcs_snapshots.lock().await;
    let entry = cache.entry(snapshot.worktree_id).or_insert_with(|| {
        crate::daemon::TimedEntry::new(crate::daemon::WorktreeVcsSnapshotCacheEntry {
            snapshot: snapshot.clone(),
            fingerprint: String::new(),
            emitted_at: now - Duration::from_millis(GIT_STATUS_MAX_INTERVAL_MS + 1),
            last_change_at: now - Duration::from_millis(GIT_STATUS_MAX_INTERVAL_MS + 1),
            last_summary_at: None,
        })
    });
    entry.touch_at(now);
    let is_first = entry.value.fingerprint.is_empty();
    if entry.value.fingerprint == fingerprint && !force_emit {
        return None;
    }
    let since_change = now.duration_since(entry.value.last_change_at);
    let since_emit = now.duration_since(entry.value.emitted_at);
    let previous_snapshot = &entry.value.snapshot;
    let must_publish_state_transition = previous_snapshot.available != snapshot.available
        || previous_snapshot.unavailable_reason != snapshot.unavailable_reason
        || previous_snapshot.compute_state != snapshot.compute_state
        || previous_snapshot.freshness != snapshot.freshness
        || previous_snapshot.touched_files_state != snapshot.touched_files_state
        || (matches!(
            snapshot.touched_files_state,
            WorktreeVcsTouchedFilesState::Ready
        ) && previous_snapshot.touched_files != snapshot.touched_files);
    if !force_emit
        && !is_first
        && !must_publish_state_transition
        && since_emit < Duration::from_millis(GIT_STATUS_MAX_INTERVAL_MS)
        && since_change < Duration::from_millis(GIT_STATUS_DEBOUNCE_MS)
    {
        return None;
    }
    let next_rev = entry.value.snapshot.rev.saturating_add(1);
    snapshot.rev = next_rev;
    snapshot.emitted_at_ms = now_epoch_ms();
    if snapshot.schema_version == 0 {
        snapshot.schema_version = WORKTREE_VCS_SNAPSHOT_SCHEMA_VERSION;
    }
    entry.value.snapshot = snapshot.clone();
    entry.value.fingerprint = fingerprint;
    entry.value.emitted_at = now;
    entry.value.last_change_at = now;
    if let Some(summary_at) = summary_at {
        entry.value.last_summary_at = Some(summary_at);
    }
    Some(snapshot)
}

pub(super) async fn refresh_worktree_vcs_projection(
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
    let cached_summary = cached_snapshot
        .as_ref()
        .map(|snapshot| snapshot.summary.clone())
        .unwrap_or_default();
    let cached_touched_files = cached_snapshot
        .as_ref()
        .map(|snapshot| snapshot.touched_files.clone())
        .unwrap_or_default();
    let cached_touched_files_state = cached_snapshot
        .as_ref()
        .map(|snapshot| snapshot.touched_files_state.clone())
        .unwrap_or_default();
    let cached_available = cached_snapshot
        .as_ref()
        .map(|snapshot| snapshot.available)
        .unwrap_or(true);
    let cached_unavailable_reason = cached_snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.unavailable_reason.clone());

    if !worktree_has_vcs_repo(state, worktree).await? {
        return publish_no_repo_snapshot(
            state,
            worktree,
            crate::api::sessions::WorktreeDiffBaseResolution {
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

    let store = state.store_for_worktree(worktree.id).await?;
    let workspace = state
        .global_store()
        .get_workspace(worktree.workspace_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("workspace not found for worktree"))?;
    let resolution = resolve_diff_base_with_meta(
        state,
        &store,
        &workspace,
        worktree,
        &SessionDiffQuery::default(),
    )
    .await;
    if let Some(reason) = resolution.unavailable_reason.clone() {
        return publish_unavailable_snapshot(state, worktree, resolution, force_emit, reason).await;
    }

    let (summary, compute_state, summary_at, available, unavailable_reason) =
        if refresh_summary || !summary_has_counts(&cached_summary) {
            match load_diff_file_count(state, worktree, &resolution.base_commit_sha).await {
                Ok(file_count) => (
                    summary_from_file_count(file_count),
                    WorktreeVcsComputeState::Ready,
                    Some(Instant::now()),
                    true,
                    None,
                ),
                Err(err) if crate::api::sessions::is_no_vcs_repo_error(&err) => (
                    WorktreeVcsSummary::default(),
                    WorktreeVcsComputeState::Ready,
                    None,
                    false,
                    Some(ctx_core::models::DiffUnavailableReason::NoRepo),
                ),
                Err(err) => {
                    tracing::warn!(
                        worktree_id = %worktree.id.0,
                        "worktree diff file-count refresh failed: {err:#}"
                    );
                    (
                        cached_summary.clone(),
                        WorktreeVcsComputeState::Error,
                        None,
                        cached_available,
                        cached_unavailable_reason.clone(),
                    )
                }
            }
        } else {
            (
                cached_summary.clone(),
                WorktreeVcsComputeState::Ready,
                None,
                cached_available,
                cached_unavailable_reason.clone(),
            )
        };

    let large_change_set_file_count = summary
        .file_count
        .filter(|count| *count > WORKTREE_VCS_REVIEWABLE_FILE_LIMIT);
    let include_status_inventory = refresh_touched_files && large_change_set_file_count.is_none();
    let git_snapshot = match load_git_status_snapshot(
        state,
        worktree,
        include_status_inventory,
        include_status_inventory,
    )
    .await
    {
        Ok(snapshot) => snapshot,
        Err(err) if crate::api::sessions::is_no_vcs_repo_error(&err) => {
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

    let (touched_files, touched_files_state) = if let Some(file_count) = large_change_set_file_count
    {
        (
            build_large_change_set_touched_files(file_count),
            WorktreeVcsTouchedFilesState::Ready,
        )
    } else if refresh_touched_files {
        match load_diff_touched_entries(state, worktree, &resolution.base_commit_sha).await {
            Ok(entries) => (
                build_touched_files(&entries),
                WorktreeVcsTouchedFilesState::Ready,
            ),
            Err(err) if crate::api::sessions::is_no_vcs_repo_error(&err) => {
                return publish_no_repo_snapshot(state, worktree, resolution, force_emit).await;
            }
            Err(err) => {
                tracing::warn!(
                    worktree_id = %worktree.id.0,
                    "worktree touched-file refresh failed: {err:#}"
                );
                (
                    cached_touched_files.clone(),
                    WorktreeVcsTouchedFilesState::Error,
                )
            }
        }
    } else {
        let next_state = match cached_touched_files_state {
            WorktreeVcsTouchedFilesState::Loading => WorktreeVcsTouchedFilesState::NotLoaded,
            other => other,
        };
        (cached_touched_files.clone(), next_state)
    };

    let snapshot = build_worktree_vcs_snapshot_from_parts(
        state,
        worktree,
        git_status,
        touched_files.clone(),
        touched_files_state.clone(),
        summary.clone(),
        compute_state.clone(),
        Some(resolution),
        available,
        unavailable_reason,
    )
    .await?;

    publish_worktree_vcs_snapshot(state, worktree, snapshot, force_emit, summary_at).await;

    let mut runtime = state.workspaces.worktree_vcs_runtime.lock().await;
    let entry = runtime.entry(worktree.id).or_default();
    entry.last_git_status = Some(git_snapshot);
    entry.touched_files = touched_files;
    entry.touched_files_state = touched_files_state;
    entry.dirty_bits = crate::daemon::WorktreeVcsDirtyBits::default();
    entry.require_full_summary_rebuild = false;
    entry.candidate_paths.clear();
    Ok(())
}

pub(super) async fn publish_transient_worktree_vcs_snapshot(
    state: &Arc<AppState>,
    worktree: &Worktree,
    snapshot: WorktreeVcsSnapshot,
) {
    let Some(snapshot) = upsert_worktree_vcs_snapshot(state, snapshot, false, None).await else {
        return;
    };
    if state.is_worktree_vcs_active(worktree.id).await {
        state
            .workspaces
            .workspace_active_snapshot
            .publish_worktree_vcs_snapshot(worktree.workspace_id, snapshot)
            .await;
    }
}
