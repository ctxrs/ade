use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard as StdMutexGuard};
use std::time::Duration;

use anyhow::Result;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

use ctx_core::models::Worktree;
use ctx_fs::patch::should_ignore_path;

use crate::daemon::AppState;
use crate::settings::ExecutionMode;
use crate::worktree_data_plane::resolve_worktree_data_plane;

use super::sandbox::container_git_stdout;
use super::{mark_worktree_vcs_dirty, vcs_driver_for_worktree};

const GIT_STATUS_WATCH_DEBOUNCE_MS: u64 = 500;
const GIT_STATUS_POLL_INTERVAL_MS: u64 = 60_000;

#[derive(Default)]
struct WatchInvalidation {
    dirty_bits: crate::daemon::WorktreeVcsDirtyBits,
    candidate_paths: BTreeSet<String>,
}

#[derive(Default)]
struct WatchPendingState {
    invalidation: WatchInvalidation,
    scheduled: bool,
}

fn normalize_path_for_comparison(path: &Path) -> PathBuf {
    let mut suffix = Vec::new();
    let mut cursor = path;
    loop {
        match std::fs::canonicalize(cursor) {
            Ok(canonical) => {
                let mut normalized = canonical;
                for component in suffix.iter().rev() {
                    normalized.push(component);
                }
                return normalized;
            }
            Err(_) => {
                let Some(parent) = cursor.parent() else {
                    return path.to_path_buf();
                };
                let Some(name) = cursor.file_name() else {
                    return path.to_path_buf();
                };
                suffix.push(name.to_os_string());
                cursor = parent;
            }
        }
    }
}

fn has_invalidation(pending: &WatchInvalidation) -> bool {
    pending.dirty_bits.any() || !pending.candidate_paths.is_empty()
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
    pending: WatchInvalidation,
) {
    if !has_invalidation(&pending) {
        return;
    }
    let dirty_bits = pending.dirty_bits;
    let candidate_paths = pending.candidate_paths.into_iter().collect::<Vec<_>>();
    if let Err(err) = mark_worktree_vcs_dirty(state, worktree, dirty_bits, candidate_paths).await {
        tracing::warn!(worktree_id = %worktree.id.0, "git status invalidation failed: {err:#}");
    }
}

fn merge_invalidation(target: &mut WatchInvalidation, next: WatchInvalidation) {
    target.dirty_bits.worktree_fs |= next.dirty_bits.worktree_fs;
    target.dirty_bits.vcs_meta |= next.dirty_bits.vcs_meta;
    target.candidate_paths.extend(next.candidate_paths);
}

pub(super) async fn run_git_status_watcher(state: Arc<AppState>, worktree: Worktree) -> Result<()> {
    let data_plane = resolve_worktree_data_plane(&state, &worktree).await?;
    let root = data_plane.live_worktree_root.as_path();
    if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
        // Disk-isolated worktrees live inside the harness container; host filesystem watchers
        // cannot observe changes. Polling keeps VCS snapshots up to date.
        let _ = container_git_stdout(&state, &worktree, &["rev-parse", "--is-inside-work-tree"])
            .await?;
        return run_git_status_poller(state, worktree).await;
    }
    let vcs = vcs_driver_for_worktree(&worktree);
    vcs.assert_repo(root).await?;
    let metadata_roots = resolve_metadata_roots(&worktree, root).await?;

    let mut watcher = watcher(
        state.clone(),
        worktree.clone(),
        root.to_path_buf(),
        metadata_roots.clone(),
        Duration::from_millis(GIT_STATUS_WATCH_DEBOUNCE_MS),
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
    let resolved = if path.is_absolute() {
        path
    } else {
        worktree_root.join(path)
    };
    match tokio::fs::canonicalize(&resolved).await {
        Ok(path) => Ok(path),
        Err(_) => Ok(resolved),
    }
}

async fn resolve_common_git_dir(git_dir: &Path) -> Result<PathBuf> {
    let commondir = git_dir.join("commondir");
    let meta = match tokio::fs::symlink_metadata(&commondir).await {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(git_dir.to_path_buf());
        }
        Err(err) => return Err(err.into()),
    };
    if !meta.is_file() {
        return Ok(git_dir.to_path_buf());
    }
    let raw = tokio::fs::read_to_string(&commondir).await?;
    let path = PathBuf::from(raw.trim());
    let resolved = if path.is_absolute() {
        path
    } else {
        git_dir.join(path)
    };
    match tokio::fs::canonicalize(&resolved).await {
        Ok(path) => Ok(path),
        Err(_) => Ok(resolved),
    }
}

async fn resolve_metadata_roots(worktree: &Worktree, worktree_root: &Path) -> Result<Vec<PathBuf>> {
    match worktree
        .vcs_kind
        .clone()
        .unwrap_or(ctx_core::models::VcsKind::Git)
    {
        ctx_core::models::VcsKind::Jj => Ok(vec![worktree_root.join(".jj")]),
        _ => {
            let git_dir = resolve_git_dir(worktree_root).await?;
            let common_git_dir = resolve_common_git_dir(&git_dir).await?;
            let mut roots = vec![git_dir];
            if !roots.iter().any(|root| *root == common_git_dir) {
                roots.push(common_git_dir);
            }
            Ok(roots)
        }
    }
}

async fn run_git_status_poller(state: Arc<AppState>, worktree: Worktree) -> Result<()> {
    let mut interval = tokio::time::interval(Duration::from_millis(GIT_STATUS_POLL_INTERVAL_MS));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        if let Err(err) = mark_worktree_vcs_dirty(
            &state,
            &worktree,
            crate::daemon::WorktreeVcsDirtyBits {
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

fn should_ignore_event(event: &Event, worktree_root: &Path, metadata_roots: &[PathBuf]) -> bool {
    event.paths.iter().all(|path| {
        let normalized = normalize_path_for_comparison(path);
        if metadata_roots
            .iter()
            .any(|metadata_root| normalized.starts_with(metadata_root))
        {
            return false;
        }
        if let Ok(relative) = normalized.strip_prefix(worktree_root) {
            return !relative.as_os_str().is_empty() && should_ignore_path(relative);
        }
        should_ignore_path(&normalized)
    })
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
    let worktree_root = normalize_path_for_comparison(&worktree_root);
    let metadata_roots = metadata_roots
        .into_iter()
        .map(|path| normalize_path_for_comparison(&path))
        .collect::<Vec<_>>();
    let watcher = notify::recommended_watcher(move |res| {
        if let Ok(event) = res {
            if should_ignore_event(&event, &worktree_root, &metadata_roots) {
                return;
            }
            let mut invalidation = WatchInvalidation::default();
            for path in &event.paths {
                let normalized = normalize_path_for_comparison(path);
                if metadata_roots
                    .iter()
                    .any(|metadata_root| normalized.starts_with(metadata_root))
                {
                    invalidation.dirty_bits.vcs_meta = true;
                    continue;
                }
                if let Ok(relative) = normalized.strip_prefix(&worktree_root) {
                    let relative = relative.to_string_lossy().trim().to_string();
                    if !relative.is_empty() {
                        invalidation.candidate_paths.insert(relative);
                        invalidation.dirty_bits.worktree_fs = true;
                    }
                } else {
                    invalidation.dirty_bits.vcs_meta = true;
                }
            }
            if invalidation.dirty_bits.any() || !invalidation.candidate_paths.is_empty() {
                let should_spawn = {
                    let mut guard = lock_watch_pending(&pending);
                    merge_invalidation(&mut guard.invalidation, invalidation);
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
                            if has_invalidation(&guard.invalidation) {
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

#[cfg(test)]
mod tests {
    use super::{normalize_path_for_comparison, should_ignore_event};
    use notify::{event::EventKind, Event};
    use std::path::Path;

    #[test]
    #[cfg(unix)]
    fn should_not_ignore_metadata_event_when_event_path_uses_symlink_alias() {
        let temp = tempfile::tempdir().expect("tempdir");
        let real_root = temp.path().join("real");
        std::fs::create_dir_all(real_root.join(".git/refs/heads")).expect("create repo dirs");
        let alias_root = temp.path().join("alias");
        std::os::unix::fs::symlink(&real_root, &alias_root).expect("create alias symlink");

        let metadata_root = normalize_path_for_comparison(&real_root.join(".git"));
        let event = Event {
            kind: EventKind::Any,
            paths: vec![alias_root.join(".git/refs/heads/merge-target")],
            attrs: Default::default(),
        };

        assert!(
            !should_ignore_event(
                &event,
                &normalize_path_for_comparison(&real_root),
                &[metadata_root]
            ),
            "metadata events must survive symlink/canonical path alias differences",
        );
    }

    #[test]
    fn should_not_ignore_managed_worktree_files_under_ctx_parent() {
        let temp = tempfile::tempdir().expect("tempdir");
        let worktree_root = temp.path().join(".ctx/worktrees/workspace/task");
        std::fs::create_dir_all(worktree_root.join("src")).expect("create worktree");
        let event = Event {
            kind: EventKind::Any,
            paths: vec![worktree_root.join("src/main.rs")],
            attrs: Default::default(),
        };

        assert!(
            !should_ignore_event(&event, &normalize_path_for_comparison(&worktree_root), &[]),
            "managed worktree files under a .ctx parent must still invalidate VCS state",
        );
    }

    #[test]
    fn normalize_path_for_comparison_preserves_missing_suffix() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("repo");
        std::fs::create_dir_all(&root).expect("create root");
        let missing = root.join(".git/refs/heads/missing");

        let normalized = normalize_path_for_comparison(&missing);

        assert!(
            normalized.ends_with(Path::new(".git/refs/heads/missing")),
            "missing suffix should be preserved after normalization",
        );
    }
}
