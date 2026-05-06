use std::sync::Arc;

use ctx_workspace_services::worktree_vcs::{claim_next_worktree_vcs_job, finish_worktree_vcs_job};

use crate::daemon::AppState;

use super::projection::refresh_worktree_vcs_projection;

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
            Err(err) => Err(err),
        },
        Err(err) => Err(err),
    };

    if let Err(err) = result {
        tracing::warn!(
            worktree_id = %worktree_id.0,
            "worktree vcs scheduler refresh failed: {err:#}"
        );
    }

    let should_notify = {
        let mut runtime = state.workspaces.worktree_vcs_runtime.lock().await;
        finish_worktree_vcs_job(&mut runtime, worktree_id)
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
    claim_next_worktree_vcs_job(&mut runtime, &active, &open).map(|job| {
        (
            job.worktree_id,
            job.refresh_summary,
            job.refresh_touched_files,
        )
    })
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

pub(super) async fn ensure_worktree_vcs_scheduler_started(state: &Arc<AppState>) {
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
