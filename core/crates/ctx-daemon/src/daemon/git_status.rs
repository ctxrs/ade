use std::sync::Arc;

use anyhow::Result;

use ctx_core::models::Worktree;

use crate::daemon::DaemonState;
mod projection;
mod sandbox;
mod scheduler;
mod snapshot;
mod source;
mod watch;
use ctx_worktree_vcs_service::{
    worktree_has_vcs_repo_from_source, worktree_vcs_dirty_transient_snapshot,
    worktree_vcs_driver_for_kind, worktree_vcs_refresh_transient_snapshot, WorktreeVcsDirtyBits,
    WorktreeVcsDriver,
};
pub use projection::load_git_status_snapshot;
use projection::{publish_transient_worktree_vcs_snapshot, refresh_worktree_vcs_projection};
use scheduler::ensure_worktree_vcs_scheduler_started;
pub use source::HttpWorktreeVcsSource;

fn vcs_driver_for_worktree(worktree: &Worktree) -> Arc<WorktreeVcsDriver> {
    worktree_vcs_driver_for_kind(worktree.vcs_kind.clone())
}

pub async fn worktree_has_vcs_repo(state: &Arc<DaemonState>, worktree: &Worktree) -> Result<bool> {
    let source = source::HttpWorktreeVcsSource::new(state, worktree);
    worktree_has_vcs_repo_from_source(&source).await
}

pub async fn request_worktree_vcs_refresh(
    state: &Arc<DaemonState>,
    worktree: &Worktree,
    summary: bool,
    touched_files: bool,
) -> Result<()> {
    request_worktree_vcs_refresh_inner(state, worktree, summary, touched_files, true).await
}

pub async fn request_worktree_vcs_refresh_without_transient(
    state: &Arc<DaemonState>,
    worktree: &Worktree,
    summary: bool,
    touched_files: bool,
) -> Result<()> {
    request_worktree_vcs_refresh_inner(state, worktree, summary, touched_files, false).await
}

async fn request_worktree_vcs_refresh_inner(
    state: &Arc<DaemonState>,
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

    state
        .queue_worktree_vcs_refresh(worktree.id, summary, touched_files)
        .await;

    if publish_transient {
        if let Some(snapshot) = state.get_worktree_vcs_snapshot(worktree.id).await {
            let snapshot =
                worktree_vcs_refresh_transient_snapshot(snapshot, summary, touched_files);
            publish_transient_worktree_vcs_snapshot(state, worktree, snapshot).await;
        }
    }

    state.notify_worktree_vcs_scheduler();
    Ok(())
}

pub async fn mark_worktree_vcs_dirty(
    state: &Arc<DaemonState>,
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
    state
        .mark_worktree_vcs_runtime_dirty(worktree.id, dirty_bits, candidate_paths, pane_open)
        .await;

    if let Some(snapshot) = state.get_worktree_vcs_snapshot(worktree.id).await {
        let snapshot = worktree_vcs_dirty_transient_snapshot(snapshot);
        publish_transient_worktree_vcs_snapshot(state, worktree, snapshot).await;
    }

    request_worktree_vcs_refresh(state, worktree, true, pane_open).await
}

pub async fn refresh_worktree_vcs_summary(
    state: Arc<DaemonState>,
    worktree: Worktree,
) -> Result<()> {
    if !state.worktree_vcs_enabled() {
        return Ok(());
    }
    refresh_worktree_vcs_projection(&state, &worktree, true, false, false).await
}

pub async fn emit_worktree_vcs_snapshot_for_worktree(
    state: &Arc<DaemonState>,
    worktree: &Worktree,
    force_emit: bool,
) -> Result<()> {
    if !state.worktree_vcs_enabled() {
        return Ok(());
    }
    let refresh_touched_files = state.is_worktree_vcs_pane_open(worktree.id).await;
    refresh_worktree_vcs_projection(state, worktree, true, refresh_touched_files, force_emit).await
}

pub async fn run_git_status_watcher(state: Arc<DaemonState>, worktree: Worktree) -> Result<()> {
    if !state.worktree_vcs_enabled() {
        return Ok(());
    }
    watch::run_git_status_watcher(state, worktree).await
}
