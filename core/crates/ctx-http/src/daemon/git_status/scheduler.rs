use std::sync::Arc;

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

    let should_notify = state.finish_worktree_vcs_job(worktree_id).await;
    if should_notify {
        state.notify_worktree_vcs_scheduler();
    }
}

async fn next_worktree_vcs_job(
    state: &Arc<AppState>,
) -> Option<(ctx_core::ids::WorktreeId, bool, bool)> {
    state.claim_next_worktree_vcs_job().await.map(|job| {
        (
            job.worktree_id,
            job.refresh_summary,
            job.refresh_touched_files,
        )
    })
}

async fn run_worktree_vcs_scheduler(state: Arc<AppState>) {
    loop {
        state.wait_worktree_vcs_scheduler_notification().await;
        loop {
            let permit = match state.try_acquire_worktree_vcs_scheduler_permit() {
                Some(permit) => permit,
                None => break,
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
    if !state.mark_worktree_vcs_scheduler_started() {
        return;
    }
    let state = state.clone();
    tokio::spawn(async move {
        run_worktree_vcs_scheduler(state).await;
    });
}
