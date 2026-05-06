use std::sync::Arc;

use anyhow::Result;

use ctx_core::models::{Worktree, WorktreeVcsComputeState, WorktreeVcsTouchedFilesState};
use ctx_fs::vcs::{self, VcsDriver};

use crate::daemon::AppState;
use crate::settings::ExecutionMode;
use crate::worktree_data_plane::resolve_worktree_data_plane;
mod diff_paths;
mod projection;
mod sandbox;
mod scheduler;
mod snapshot;
#[path = "git_status_watch.rs"]
mod watch;
use ctx_workspace_services::worktree_vcs::derive_worktree_vcs_freshness;
pub use ctx_workspace_services::worktree_vcs::{GitStatusEntry, GitStatusSnapshot};
pub use projection::load_git_status_snapshot;
use projection::{publish_transient_worktree_vcs_snapshot, refresh_worktree_vcs_projection};
pub(crate) use sandbox::{worktree_merge_base, worktree_rev_parse_head, worktree_rev_parse_refs};
use scheduler::ensure_worktree_vcs_scheduler_started;

const GIT_STATUS_DEBOUNCE_MS: u64 = 500;
const GIT_STATUS_MAX_INTERVAL_MS: u64 = 2000;

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

pub async fn request_worktree_vcs_refresh(
    state: &Arc<AppState>,
    worktree: &Worktree,
    summary: bool,
    touched_files: bool,
) -> Result<()> {
    request_worktree_vcs_refresh_inner(state, worktree, summary, touched_files, true).await
}

pub(crate) async fn request_worktree_vcs_refresh_without_transient(
    state: &Arc<AppState>,
    worktree: &Worktree,
    summary: bool,
    touched_files: bool,
) -> Result<()> {
    request_worktree_vcs_refresh_inner(state, worktree, summary, touched_files, false).await
}

async fn request_worktree_vcs_refresh_inner(
    state: &Arc<AppState>,
    worktree: &Worktree,
    summary: bool,
    touched_files: bool,
    publish_transient: bool,
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

    if publish_transient {
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
