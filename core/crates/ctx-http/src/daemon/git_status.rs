use std::sync::Arc;

use anyhow::Result;

use ctx_core::models::Worktree;

use crate::daemon::AppState;
mod projection;
mod sandbox;
mod scheduler;
mod snapshot;
mod source;
mod watch;
use ctx_workspace_services::worktree_vcs::{
    mark_worktree_vcs_runtime_dirty, queue_worktree_vcs_refresh, worktree_has_vcs_repo_from_source,
    worktree_vcs_dirty_transient_snapshot, worktree_vcs_driver_for_kind,
    worktree_vcs_refresh_transient_snapshot, WorktreeVcsDriver,
};
pub use ctx_workspace_services::worktree_vcs::{
    GitStatusEntry, GitStatusSnapshot, WorktreeVcsDirtyBits,
};
pub use projection::load_git_status_snapshot;
use projection::{publish_transient_worktree_vcs_snapshot, refresh_worktree_vcs_projection};
use scheduler::ensure_worktree_vcs_scheduler_started;
pub(crate) use source::HttpWorktreeVcsSource;

fn vcs_driver_for_worktree(worktree: &Worktree) -> Arc<WorktreeVcsDriver> {
    worktree_vcs_driver_for_kind(worktree.vcs_kind.clone())
}

pub(crate) async fn worktree_has_vcs_repo(
    state: &Arc<AppState>,
    worktree: &Worktree,
) -> Result<bool> {
    let source = source::HttpWorktreeVcsSource::new(state, worktree);
    worktree_has_vcs_repo_from_source(&source).await
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
        queue_worktree_vcs_refresh(entry, summary, touched_files);
    }

    if publish_transient {
        if let Some(snapshot) = state.get_worktree_vcs_snapshot(worktree.id).await {
            let snapshot =
                worktree_vcs_refresh_transient_snapshot(snapshot, summary, touched_files);
            publish_transient_worktree_vcs_snapshot(state, worktree, snapshot).await;
        }
    }

    state.workspaces.worktree_vcs_scheduler.notify.notify_one();
    Ok(())
}

pub async fn mark_worktree_vcs_dirty(
    state: &Arc<AppState>,
    worktree: &Worktree,
    dirty_bits: WorktreeVcsDirtyBits,
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
        mark_worktree_vcs_runtime_dirty(entry, dirty_bits, candidate_paths, pane_open);
    }

    if let Some(snapshot) = state.get_worktree_vcs_snapshot(worktree.id).await {
        let snapshot = worktree_vcs_dirty_transient_snapshot(snapshot);
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
