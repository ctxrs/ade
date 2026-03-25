use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use ctx_core::models::Worktree;
use ctx_fs::patch::should_ignore_path;

use crate::daemon::AppState;
use crate::settings::ExecutionMode;
use crate::worktree_data_plane::resolve_worktree_data_plane;

use super::sandbox::container_git_stdout;
use super::{emit_worktree_vcs_snapshot_for_worktree, vcs_driver_for_worktree};

const GIT_STATUS_WATCH_DEBOUNCE_MS: u64 = 500;
const GIT_STATUS_POLL_INTERVAL_MS: u64 = 60_000;

pub(super) async fn run_git_status_watcher(state: Arc<AppState>, worktree: Worktree) -> Result<()> {
    let data_plane = resolve_worktree_data_plane(&state, &worktree).await?;
    let root = data_plane.live_worktree_root.as_path();
    if matches!(data_plane.execution_mode, ExecutionMode::Container) {
        // Disk-isolated worktrees live inside the harness container; host filesystem watchers
        // cannot observe changes. Polling keeps VCS snapshots up to date.
        let _ = container_git_stdout(&state, &worktree, &["rev-parse", "--is-inside-work-tree"])
            .await?;
        return run_git_status_poller(state, worktree).await;
    }
    let vcs = vcs_driver_for_worktree(&worktree);
    vcs.assert_repo(root).await?;
    let git_dir = resolve_git_dir(root).await?;

    let (tx, mut rx) = mpsc::channel::<()>(1);
    let mut watcher = watcher(tx, git_dir.clone())?;
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
    if !git_dir.starts_with(root) {
        if let Err(err) = watcher.watch(&git_dir, RecursiveMode::Recursive) {
            tracing::warn!(
                worktree_id = %worktree.id.0,
                git_dir = %git_dir.display(),
                "shared git-dir watcher unavailable; falling back to polling: {err:#}"
            );
            return run_git_status_poller(state, worktree).await;
        }
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

async fn resolve_git_dir(worktree_root: &Path) -> Result<PathBuf> {
    let dotgit = worktree_root.join(".git");
    let meta = tokio::fs::metadata(&dotgit).await?;
    if meta.is_dir() {
        return Ok(dotgit);
    }
    let txt = tokio::fs::read_to_string(&dotgit).await?;
    let line = txt
        .lines()
        .find(|l| l.trim_start().starts_with("gitdir:"))
        .ok_or_else(|| anyhow::anyhow!("invalid .git file: missing gitdir"))?;
    let raw = line.trim_start().trim_start_matches("gitdir:").trim();
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(worktree_root.join(path))
    }
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

fn should_ignore_event(event: &Event, git_dir: &Path) -> bool {
    event.paths.iter().all(|path| {
        if path.starts_with(git_dir) {
            return false;
        }
        should_ignore_path(path)
    })
}

fn watcher(tx: mpsc::Sender<()>, git_dir: PathBuf) -> Result<RecommendedWatcher> {
    let watcher = notify::recommended_watcher(move |res| {
        if let Ok(event) = res {
            if should_ignore_event(&event, &git_dir) {
                return;
            }
            let _ = tx.try_send(());
        }
    })?;
    Ok(watcher)
}
