use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex, MutexGuard as StdMutexGuard};
use std::time::Duration;

use anyhow::Result;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

use ctx_core::models::Worktree;
use ctx_workspace_services::worktree_vcs::{
    normalize_worktree_vcs_watch_path, resolve_worktree_vcs_metadata_roots,
    worktree_vcs_invalidation_for_watch_paths, WorktreeVcsDirtyBits, WorktreeVcsGitCommand,
    WorktreeVcsInvalidation, WORKTREE_VCS_POLL_INTERVAL_MS, WORKTREE_VCS_WATCH_DEBOUNCE_MS,
};

use crate::daemon::AppState;
use crate::settings::ExecutionMode;
use ctx_worktree_data_plane::resolve_worktree_data_plane_with_host as resolve_worktree_data_plane;

use super::sandbox::container_git_stdout;
use super::{mark_worktree_vcs_dirty, vcs_driver_for_worktree};

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

pub(super) async fn run_git_status_watcher(state: Arc<AppState>, worktree: Worktree) -> Result<()> {
    let data_plane = resolve_worktree_data_plane(state.as_ref(), &worktree).await?;
    let root = data_plane.live_worktree_root.as_path();
    if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
        // Disk-isolated worktrees live inside the harness container; host filesystem watchers
        // cannot observe changes. Polling keeps VCS snapshots up to date.
        let _ = container_git_stdout(&state, &worktree, WorktreeVcsGitCommand::IsInsideWorkTree)
            .await?;
        return run_git_status_poller(state, worktree).await;
    }
    let vcs = vcs_driver_for_worktree(&worktree);
    vcs.assert_repo(root).await?;
    let metadata_roots = resolve_worktree_vcs_metadata_roots(&worktree, root).await?;

    let mut watcher = watcher(
        state.clone(),
        worktree.clone(),
        root.to_path_buf(),
        metadata_roots.clone(),
        Duration::from_millis(WORKTREE_VCS_WATCH_DEBOUNCE_MS),
    )?;
    if let Err(err) = watcher.watch(root, RecursiveMode::Recursive) {
        // On hosts with low watch limits (or many concurrent watchers), file watching can fail with
        // ENOSPC/too-many-watches. Falling back to polling keeps git status updates flowing and
        // avoids flaking tests that rely on live status changes.
        tracing::warn!(
            worktree_id = %worktree.id.0,
            "git status watcher unavailable; falling back to polling: {err:#}"
        );
        return run_git_status_poller(state, worktree).await;
    }
    for metadata_root in metadata_roots
        .iter()
        .filter(|metadata_root| !metadata_root.starts_with(root))
    {
        if let Err(err) = watcher.watch(metadata_root, RecursiveMode::Recursive) {
            tracing::warn!(
                worktree_id = %worktree.id.0,
                metadata_root = %metadata_root.display(),
                "vcs metadata watcher unavailable; falling back to polling: {err:#}"
            );
            return run_git_status_poller(state, worktree).await;
        }
    }
    std::future::pending::<Result<()>>().await
}

async fn run_git_status_poller(state: Arc<AppState>, worktree: Worktree) -> Result<()> {
    let mut interval = tokio::time::interval(Duration::from_millis(WORKTREE_VCS_POLL_INTERVAL_MS));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        if let Err(err) = mark_worktree_vcs_dirty(
            &state,
            &worktree,
            WorktreeVcsDirtyBits {
                worktree_fs: true,
                vcs_meta: true,
            },
            Vec::new(),
        )
        .await
        {
            tracing::warn!(worktree_id = %worktree.id.0, "git status invalidation failed: {err:#}");
        }
    }
}

fn watcher(
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
