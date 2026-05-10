use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex, MutexGuard as StdMutexGuard};
use std::time::Duration;

use anyhow::Result;
use ctx_core::models::Worktree;
use ctx_workspace_services::worktree_vcs::{
    normalize_worktree_vcs_watch_path, worktree_vcs_invalidation_for_watch_paths,
    WorktreeVcsInvalidation,
};
use notify::{Event, RecommendedWatcher};

use crate::daemon::AppState;

use super::super::mark_worktree_vcs_dirty;

#[derive(Default)]
struct WatchPendingState {
    invalidation: WorktreeVcsInvalidation,
    scheduled: bool,
}

fn lock_watch_pending<'a>(
    pending: &'a Arc<StdMutex<WatchPendingState>>,
) -> StdMutexGuard<'a, WatchPendingState> {
    match pending.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!(
                "git status watcher pending-state mutex was poisoned; continuing with inner state"
            );
            poisoned.into_inner()
        }
    }
}

async fn dispatch_invalidation(
    state: &Arc<AppState>,
    worktree: &Worktree,
    pending: WorktreeVcsInvalidation,
) {
    if !pending.any() {
        return;
    }
    let (dirty_bits, candidate_paths) = pending.into_parts();
    if let Err(err) = mark_worktree_vcs_dirty(state, worktree, dirty_bits, candidate_paths).await {
        tracing::warn!(worktree_id = %worktree.id.0, "git status invalidation failed: {err:#}");
    }
}

pub(super) fn build_git_status_watcher(
    state: Arc<AppState>,
    worktree: Worktree,
    worktree_root: PathBuf,
    metadata_roots: Vec<PathBuf>,
    debounce: Duration,
) -> Result<RecommendedWatcher> {
    let handle = tokio::runtime::Handle::current();
    let pending = Arc::new(StdMutex::new(WatchPendingState::default()));
    let worktree_root = normalize_worktree_vcs_watch_path(&worktree_root);
    let metadata_roots = metadata_roots
        .into_iter()
        .map(|path| normalize_worktree_vcs_watch_path(&path))
        .collect::<Vec<_>>();
    let watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
        if let Ok(event) = res {
            let invalidation = worktree_vcs_invalidation_for_watch_paths(
                &event.paths,
                &worktree_root,
                &metadata_roots,
            );
            if invalidation.any() {
                let should_spawn = {
                    let mut guard = lock_watch_pending(&pending);
                    guard.invalidation.merge(invalidation);
                    if guard.scheduled {
                        false
                    } else {
                        guard.scheduled = true;
                        true
                    }
                };
                if should_spawn {
                    let pending = pending.clone();
                    let state = state.clone();
                    let worktree = worktree.clone();
                    let handle = handle.clone();
                    handle.spawn(async move {
                        loop {
                            tokio::time::sleep(debounce).await;
                            let next = {
                                let mut guard = lock_watch_pending(&pending);
                                std::mem::take(&mut guard.invalidation)
                            };
                            dispatch_invalidation(&state, &worktree, next).await;
                            let mut guard = lock_watch_pending(&pending);
                            if guard.invalidation.any() {
                                continue;
                            }
                            guard.scheduled = false;
                            break;
                        }
                    });
                }
            }
        }
    })?;
    Ok(watcher)
}
