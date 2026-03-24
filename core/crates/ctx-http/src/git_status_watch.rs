use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use ctx_core::models::Worktree;
use ctx_fs::patch::should_ignore_path;

use crate::daemon::AppState;

use super::{
    container_git_stdout, emit_worktree_vcs_snapshot_for_worktree, vcs_driver_for_worktree,
};

const GIT_STATUS_WATCH_DEBOUNCE_MS: u64 = 500;
const GIT_STATUS_POLL_INTERVAL_MS: u64 = 60_000;

pub(super) async fn run_git_status_watcher(state: Arc<AppState>, worktree: Worktree) -> Result<()> {
    let root = Path::new(&worktree.root_path);
    if crate::container_fs::is_container_path(root) {
        // Disk-isolated worktrees live inside the harness container; host filesystem watchers
        // cannot observe changes. Polling keeps VCS snapshots up to date.
        let _ = container_git_stdout(&state, &worktree, &["rev-parse", "--is-inside-work-tree"])
            .await?;
        return run_git_status_poller(state, worktree).await;
    }
    let vcs = vcs_driver_for_worktree(&worktree);
    vcs.assert_repo(root).await?;

    let (tx, mut rx) = mpsc::channel::<()>(1);
    let mut watcher = watcher(tx)?;
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

    let debounce = Duration::from_millis(GIT_STATUS_WATCH_DEBOUNCE_MS);
    let mut pending = false;
    let timer = tokio::time::sleep(debounce);
    tokio::pin!(timer);

    loop {
        tokio::select! {
            signal = rx.recv() => {
                let Some(()) = signal else {
                    break;
                };
                pending = true;
                timer.as_mut().reset(tokio::time::Instant::now() + debounce);
            }
            _ = &mut timer, if pending => {
                pending = false;
                if let Err(err) = emit_worktree_vcs_snapshot_for_worktree(&state, &worktree, false).await {
                    tracing::warn!(worktree_id = %worktree.id.0, "git status snapshot failed: {err:#}");
                }
            }
        }
    }
    Ok(())
}

async fn run_git_status_poller(state: Arc<AppState>, worktree: Worktree) -> Result<()> {
    let mut interval = tokio::time::interval(Duration::from_millis(GIT_STATUS_POLL_INTERVAL_MS));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        if let Err(err) = emit_worktree_vcs_snapshot_for_worktree(&state, &worktree, false).await {
            tracing::warn!(worktree_id = %worktree.id.0, "git status snapshot failed: {err:#}");
        }
    }
}

fn should_ignore_event(event: &Event) -> bool {
    event.paths.iter().all(|path| should_ignore_path(path))
}

fn watcher(tx: mpsc::Sender<()>) -> Result<RecommendedWatcher> {
    let watcher = notify::recommended_watcher(move |res| {
        if let Ok(event) = res {
            if should_ignore_event(&event) {
                return;
            }
            let _ = tx.try_send(());
        }
    })?;
    Ok(watcher)
}
