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
