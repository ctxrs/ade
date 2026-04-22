use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde::Serialize;

use ctx_core::models::{
    Worktree, WorktreeVcsBaseResolution, WorktreeVcsBaseResolutionKind, WorktreeVcsComputeState,
    WorktreeVcsFreshness, WorktreeVcsGitStatusSummary, WorktreeVcsSnapshot, WorktreeVcsSummary,
    WorktreeVcsTouchedFile, WorktreeVcsTouchedFiles, WorktreeVcsTouchedFilesState,
};
use ctx_fs::vcs::{self, VcsDriver};

use crate::api::sessions::{resolve_diff_base_with_meta, SessionDiffQuery};
use crate::daemon::AppState;
use crate::settings::ExecutionMode;
use crate::worktree_data_plane::resolve_worktree_data_plane;
mod sandbox;
#[path = "git_status_watch.rs"]
mod watch;
use sandbox::{
    container_git_count_untracked, container_git_diff_name_status,
    container_git_diff_name_status_count, container_git_list_untracked, container_git_rev_parse,
    container_git_status_structured,
};
pub(crate) use sandbox::{worktree_merge_base, worktree_rev_parse_head};

const GIT_STATUS_DEBOUNCE_MS: u64 = 500;
const GIT_STATUS_MAX_INTERVAL_MS: u64 = 2000;
const WORKTREE_VCS_TOUCHED_FILES_CAP: usize = 200;
const WORKTREE_VCS_SNAPSHOT_SCHEMA_VERSION: i64 = 2;

#[derive(Debug, Clone, Serialize)]
pub struct GitStatusSnapshot {
    pub raw: String,
    pub summary_line: String,
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: i64,
    pub behind: i64,
    pub detached: bool,
    pub staged: i64,
    pub unstaged: i64,
    pub untracked: i64,
    pub entries: Vec<GitStatusEntry>,
    pub entries_total_count: i64,
    pub entries_truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct GitStatusEntry {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orig_path: Option<String>,
    pub index_status: String,
    pub worktree_status: String,
}

fn vcs_driver_for_worktree(worktree: &Worktree) -> Arc<dyn VcsDriver> {
    vcs::driver_for_kind(worktree.vcs_kind.clone())
}

pub(crate) async fn worktree_has_vcs_repo(
    state: &Arc<AppState>,
    worktree: &Worktree,
) -> Result<bool> {
    let data_plane = resolve_worktree_data_plane(state, worktree).await?;
    if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
        return match sandbox::container_git_stdout(
            state,
            worktree,
            &["rev-parse", "--is-inside-work-tree"],
        )
        .await
        {
            Ok(_) => Ok(true),
            Err(err) if crate::api::sessions::is_no_vcs_repo_error(&err) => Ok(false),
            Err(err) => Err(err),
        };
    }

    let root = data_plane.live_worktree_root.as_path();
    let driver = match vcs::driver_for_path(root).await {
        Ok(driver) => driver,
        Err(err) if crate::api::sessions::is_no_vcs_repo_error(&err) => return Ok(false),
        Err(err) => return Err(err),
    };
    match driver.assert_repo(root).await {
        Ok(()) => Ok(true),
        Err(err) if crate::api::sessions::is_no_vcs_repo_error(&err) => Ok(false),
        Err(err) => Err(err),
    }
}

fn now_epoch_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as i64)
        .unwrap_or(0)
}

fn snapshot_fingerprint(snapshot: &WorktreeVcsSnapshot) -> String {
    let mut copy = snapshot.clone();
    copy.rev = 0;
    copy.emitted_at_ms = 0;
    serde_json::to_string(&copy).unwrap_or_default()
}

fn build_touched_files(entries: &[WorktreeVcsTouchedFile]) -> WorktreeVcsTouchedFiles {
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

fn build_git_status_entries(entries: &[GitStatusEntry]) -> Vec<WorktreeVcsTouchedFile> {
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

fn build_git_status_summary(
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

fn summary_from_file_count(file_count: i64) -> WorktreeVcsSummary {
    WorktreeVcsSummary {
        file_count: Some(file_count),
        line_additions: None,
        line_deletions: None,
        line_count: None,
    }
}

fn snapshot_for_durable_cache(snapshot: &WorktreeVcsSnapshot) -> WorktreeVcsSnapshot {
    let mut durable = snapshot.clone();
    durable.touched_files = WorktreeVcsTouchedFiles::default();
    durable.touched_files_state = WorktreeVcsTouchedFilesState::NotLoaded;
    durable.git_status.entries.clear();
    durable
}

async fn load_diff_touched_entries(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
) -> Result<Vec<WorktreeVcsTouchedFile>> {
    let paths = load_diff_path_states(state, worktree, base_commit_sha).await?;
    let mut items = Vec::new();
    for (path, orig_path, status) in paths {
        items.push(WorktreeVcsTouchedFile {
            path,
            orig_path,
            index_status: Some(status),
            worktree_status: None,
        });
    }
    Ok(items)
}

async fn load_diff_file_count(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
) -> Result<i64> {
    let data_plane = resolve_worktree_data_plane(state, worktree).await?;
    if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
        let tracked =
            container_git_diff_name_status_count(state, worktree, base_commit_sha).await?;
        let untracked = container_git_count_untracked(state, worktree).await?;
        return Ok(tracked + untracked);
    }
    let root = data_plane.live_worktree_root.as_path();
    let driver = vcs_driver_for_worktree(worktree);
    let tracked = driver.diff_file_count(root, base_commit_sha).await?;
    let untracked = driver.untracked_file_count(root).await?;
    Ok(tracked + untracked)
}

async fn load_diff_path_states(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
) -> Result<Vec<(String, Option<String>, String)>> {
    let data_plane = resolve_worktree_data_plane(state, worktree).await?;
    let root = data_plane.live_worktree_root.as_path();
    let entries: Vec<(String, String, Option<String>)> =
        if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
            container_git_diff_name_status(state, worktree, base_commit_sha).await?
        } else {
            let driver = vcs_driver_for_worktree(worktree);
            driver
                .diff_name_status(root, base_commit_sha)
                .await?
                .into_iter()
                .map(|entry| (entry.status, entry.path, entry.orig_path))
                .collect()
        };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (status, path, orig_path) in entries {
        let path = path.trim().to_string();
        if path.is_empty() {
            continue;
        }
        if !seen.insert(path.clone()) {
            continue;
        }
        let Some(status_kind) = status.chars().next() else {
            anyhow::bail!("vcs diff returned an empty status for {path}");
        };
        out.push((path, orig_path, status_kind.to_string()));
    }
    let untracked = if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
        container_git_list_untracked(state, worktree).await?
    } else {
        let driver = vcs_driver_for_worktree(worktree);
        driver.list_untracked(root).await?
    };
    for path in untracked {
        let path = path.trim().to_string();
        if path.is_empty() {
            continue;
        }
        if !seen.insert(path.clone()) {
            continue;
        }
        out.push((path, None, "?".to_string()));
    }
    out.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
async fn build_worktree_vcs_snapshot_from_parts(
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
    let (head_commit_sha, target_branch_commit_sha) = if matches!(
        unavailable_reason,
        Some(ctx_core::models::DiffUnavailableReason::NoRepo)
    ) {
        (base_commit_sha.clone(), None)
    } else {
        let data_plane = resolve_worktree_data_plane(state, worktree).await?;
        let root = data_plane.live_worktree_root.as_path();
        if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
            let head = container_git_rev_parse(state, worktree, "HEAD").await?;
            let target = match resolution.target_branch.as_ref() {
                Some(target_branch) => {
                    Some(container_git_rev_parse(state, worktree, target_branch).await?)
                }
                None => None,
            };
            (head, target)
        } else {
            let driver = vcs::driver_for_path(root).await?;
            let head = driver.rev_parse_head(root).await?;
            let target = match resolution.target_branch.as_ref() {
                Some(target_branch) => Some(driver.rev_parse_ref(root, target_branch).await?),
                None => None,
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
        target_branch: resolution.target_branch,
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

async fn publish_no_repo_snapshot(
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

async fn publish_unavailable_snapshot(
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

fn summary_has_counts(summary: &WorktreeVcsSummary) -> bool {
    summary.file_count.is_some()
        || summary.line_additions.is_some()
        || summary.line_deletions.is_some()
        || summary.line_count.is_some()
}

fn derive_worktree_vcs_freshness(
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

pub async fn load_git_status_snapshot(
    state: &Arc<AppState>,
    worktree: &Worktree,
    include_untracked_files: bool,
) -> Result<GitStatusSnapshot> {
    let data_plane = resolve_worktree_data_plane(state, worktree).await?;
    let root = data_plane.live_worktree_root.as_path();
    let structured = if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
        container_git_status_structured(state, worktree, include_untracked_files).await?
    } else {
        let vcs = vcs_driver_for_worktree(worktree);
        vcs.status_structured(root, include_untracked_files).await?
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
        entries: structured
            .entries
            .into_iter()
            .map(|entry| GitStatusEntry {
                path: entry.path,
                orig_path: entry.orig_path,
                index_status: entry.index_status,
                worktree_status: entry.worktree_status,
            })
            .collect(),
        entries_total_count: structured.total_count,
        entries_truncated: structured.truncated,
    })
}

async fn persist_worktree_vcs_snapshot(
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

async fn publish_worktree_vcs_snapshot(
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

async fn upsert_worktree_vcs_snapshot(
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
    if !force_emit
        && !is_first
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

async fn refresh_worktree_vcs_projection(
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

    let git_snapshot = match load_git_status_snapshot(state, worktree, refresh_touched_files).await
    {
        Ok(snapshot) => snapshot,
        Err(err) if crate::api::sessions::is_no_vcs_repo_error(&err) => {
            return publish_no_repo_snapshot(state, worktree, resolution, force_emit).await;
        }
        Err(err) => return Err(err),
    };

    let git_status = build_git_status_summary(
        &git_snapshot,
        if refresh_touched_files {
            build_git_status_entries(&git_snapshot.entries)
        } else {
            Vec::new()
        },
    );

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

    let (touched_files, touched_files_state) = if refresh_touched_files {
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

async fn publish_transient_worktree_vcs_snapshot(
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

async fn run_worktree_vcs_job(
    state: Arc<AppState>,
    worktree_id: ctx_core::ids::WorktreeId,
    refresh_summary: bool,
    refresh_touched_files: bool,
) {
    let result = match state.store_for_worktree(worktree_id).await {
        Ok(store) => match store.get_worktree(worktree_id).await {
            Ok(Some(worktree)) => {
                refresh_worktree_vcs_projection(
                    &state,
                    &worktree,
                    refresh_summary,
                    refresh_touched_files,
                    true,
                )
                .await
            }
            Ok(None) => Ok(()),
            Err(err) => Err(err.into()),
        },
        Err(err) => Err(err.into()),
    };

    if let Err(err) = result {
        tracing::warn!(
            worktree_id = %worktree_id.0,
            "worktree vcs scheduler refresh failed: {err:#}"
        );
    }

    let should_notify = {
        let mut runtime = state.workspaces.worktree_vcs_runtime.lock().await;
        match runtime.get_mut(&worktree_id) {
            Some(entry) => {
                entry.running = false;
                entry.pending_summary || entry.pending_touched_files
            }
            None => false,
        }
    };
    if should_notify {
        state.workspaces.worktree_vcs_scheduler.notify.notify_one();
    }
}

async fn next_worktree_vcs_job(
    state: &Arc<AppState>,
) -> Option<(ctx_core::ids::WorktreeId, bool, bool)> {
    let active = state.workspaces.worktree_vcs_active.lock().await;
    let open = state.workspaces.worktree_vcs_open_panes.lock().await;
    let mut runtime = state.workspaces.worktree_vcs_runtime.lock().await;
    let mut selected: Option<(ctx_core::ids::WorktreeId, u8)> = None;
    for (worktree_id, entry) in runtime.iter() {
        if entry.running || (!entry.pending_summary && !entry.pending_touched_files) {
            continue;
        }
        if active.get(worktree_id).copied().unwrap_or(0) == 0 {
            continue;
        }
        let pane_open = open.get(worktree_id).copied().unwrap_or(0) > 0;
        let priority = if entry.pending_touched_files && pane_open {
            0
        } else if entry.pending_summary && pane_open {
            1
        } else if entry.pending_summary {
            2
        } else {
            3
        };
        match selected {
            Some((current_id, current_priority))
                if current_priority < priority
                    || (current_priority == priority && current_id.0 <= worktree_id.0) => {}
            _ => selected = Some((*worktree_id, priority)),
        }
    }
    let (worktree_id, _) = selected?;
    let entry = runtime.get_mut(&worktree_id)?;
    let refresh_summary = entry.pending_summary;
    let refresh_touched_files = entry.pending_touched_files;
    entry.pending_summary = false;
    entry.pending_touched_files = false;
    entry.running = true;
    Some((worktree_id, refresh_summary, refresh_touched_files))
}

async fn run_worktree_vcs_scheduler(state: Arc<AppState>) {
    loop {
        state
            .workspaces
            .worktree_vcs_scheduler
            .notify
            .notified()
            .await;
        loop {
            let permit = match state
                .workspaces
                .worktree_vcs_scheduler
                .permits
                .clone()
                .try_acquire_owned()
            {
                Ok(permit) => permit,
                Err(_) => break,
            };
            let Some((worktree_id, refresh_summary, refresh_touched_files)) =
                next_worktree_vcs_job(&state).await
            else {
                drop(permit);
                break;
            };
            let state = state.clone();
            tokio::spawn(async move {
                let _permit = permit;
                run_worktree_vcs_job(state, worktree_id, refresh_summary, refresh_touched_files)
                    .await;
            });
        }
    }
}

async fn ensure_worktree_vcs_scheduler_started(state: &Arc<AppState>) {
    let started = state
        .workspaces
        .worktree_vcs_scheduler
        .started
        .swap(true, std::sync::atomic::Ordering::AcqRel);
    if started {
        return;
    }
    let state = state.clone();
    tokio::spawn(async move {
        run_worktree_vcs_scheduler(state).await;
    });
}

pub async fn request_worktree_vcs_refresh(
    state: &Arc<AppState>,
    worktree: &Worktree,
    summary: bool,
    touched_files: bool,
) -> Result<()> {
    if !state.worktree_vcs_enabled() {
        return Ok(());
    }
    if !state.is_worktree_vcs_active(worktree.id).await {
        return Ok(());
    }
    ensure_worktree_vcs_scheduler_started(state).await;

    {
        let mut runtime = state.workspaces.worktree_vcs_runtime.lock().await;
        let entry = runtime.entry(worktree.id).or_default();
        entry.pending_summary |= summary;
        entry.pending_touched_files |= touched_files;
    }

    if let Some(mut snapshot) = state.get_worktree_vcs_snapshot(worktree.id).await {
        if summary {
            snapshot.compute_state = WorktreeVcsComputeState::Computing;
            snapshot.freshness = derive_worktree_vcs_freshness(
                &WorktreeVcsComputeState::Computing,
                &snapshot.summary,
            );
        }
        if touched_files {
            snapshot.touched_files_state = match snapshot.touched_files_state {
                WorktreeVcsTouchedFilesState::Ready => WorktreeVcsTouchedFilesState::Stale,
                WorktreeVcsTouchedFilesState::Stale => WorktreeVcsTouchedFilesState::Stale,
                _ => WorktreeVcsTouchedFilesState::Loading,
            };
        }
        publish_transient_worktree_vcs_snapshot(state, worktree, snapshot).await;
    }

    state.workspaces.worktree_vcs_scheduler.notify.notify_one();
    Ok(())
}

pub async fn mark_worktree_vcs_dirty(
    state: &Arc<AppState>,
    worktree: &Worktree,
    dirty_bits: crate::daemon::WorktreeVcsDirtyBits,
    candidate_paths: Vec<String>,
) -> Result<()> {
    if !state.worktree_vcs_enabled() {
        return Ok(());
    }
    if !state.is_worktree_vcs_active(worktree.id).await {
        return Ok(());
    }
    let pane_open = state.is_worktree_vcs_pane_open(worktree.id).await;
    {
        let mut runtime = state.workspaces.worktree_vcs_runtime.lock().await;
        let entry = runtime.entry(worktree.id).or_default();
        entry.generation = entry.generation.saturating_add(1);
        entry.dirty_bits.worktree_fs |= dirty_bits.worktree_fs;
        entry.dirty_bits.vcs_meta |= dirty_bits.vcs_meta;
        entry.require_full_summary_rebuild |= dirty_bits.vcs_meta;
        for path in candidate_paths {
            let trimmed = path.trim();
            if !trimmed.is_empty() {
                entry.candidate_paths.insert(trimmed.to_string());
            }
        }
        entry.pending_summary = true;
        entry.pending_touched_files |= pane_open;
    }

    if let Some(mut snapshot) = state.get_worktree_vcs_snapshot(worktree.id).await {
        snapshot.compute_state = WorktreeVcsComputeState::Computing;
        snapshot.freshness =
            derive_worktree_vcs_freshness(&WorktreeVcsComputeState::Computing, &snapshot.summary);
        if matches!(
            snapshot.touched_files_state,
            WorktreeVcsTouchedFilesState::Ready
        ) {
            snapshot.touched_files_state = WorktreeVcsTouchedFilesState::Stale;
        }
        publish_transient_worktree_vcs_snapshot(state, worktree, snapshot).await;
    }

    request_worktree_vcs_refresh(state, worktree, true, pane_open).await
}

pub async fn refresh_worktree_vcs_summary(state: Arc<AppState>, worktree: Worktree) -> Result<()> {
    if !state.worktree_vcs_enabled() {
        return Ok(());
    }
    refresh_worktree_vcs_projection(&state, &worktree, true, false, false).await
}

pub async fn emit_worktree_vcs_snapshot_for_worktree(
    state: &Arc<AppState>,
    worktree: &Worktree,
    force_emit: bool,
) -> Result<()> {
    if !state.worktree_vcs_enabled() {
        return Ok(());
    }
    let refresh_touched_files = state.is_worktree_vcs_pane_open(worktree.id).await;
    refresh_worktree_vcs_projection(state, worktree, true, refresh_touched_files, force_emit).await
}

pub async fn run_git_status_watcher(state: Arc<AppState>, worktree: Worktree) -> Result<()> {
    if !state.worktree_vcs_enabled() {
        return Ok(());
    }
    watch::run_git_status_watcher(state, worktree).await
}
