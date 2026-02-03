use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
#[cfg(target_os = "linux")]
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use chrono::Utc;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use ctx_core::ids::{MergeQueueEntryId, MergeQueueRunId, SessionId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    MergeQueueEntry, MergeQueueEntryStatus, MergeQueuePatchSource, MergeQueueRun,
    MergeQueueRunStatus, SessionEventType, VcsKind, Workspace, Worktree,
};
use ctx_fs::git::{delete_branch, git_status_porcelain, rev_parse_ref};
use ctx_fs::vcs::{self, ApplyPatchTarget, VcsDriver};
use ctx_fs::worktrees::{create_worktree, remove_worktree};

use crate::daemon::AppState;
use crate::ops_events::OpsEvent;
use crate::settings::{self, NetworkContext};
#[cfg(target_os = "linux")]
use crate::tool_cgroup::TOOL_SLICE_UNIT;
#[cfg(not(target_os = "linux"))]
const TOOL_SLICE_UNIT: &str = "ctx-tools.slice";
use crate::workspace_config::{load_merge_queue_config, MergeQueueCanonicalSync, MergeQueueConfig};

#[derive(Debug, Clone)]
pub struct MergeQueueSubmitParams {
    pub session_id: Option<SessionId>,
    pub worktree_id: Option<WorktreeId>,
    pub worktree_root: Option<String>,
    pub target_branch: Option<String>,
    pub message: Option<String>,
}

const MERGE_QUEUE_CANONICAL_REMOTE: &str = "canonical";
const MERGE_QUEUE_HEAD_REF: &str = "refs/heads/ctx-merge-queue";
const MERGE_QUEUE_CONFLICT_MESSAGE: &str = concat!(
    "Your merge queue submission produces conflicts with the current head. ",
    "Please rebase your changes, carefully considering the intent of your changes and the intent of the upstream changes. ",
    "If in doubt about how to resolve conflicts, please ask for help."
);

fn vcs_driver_for_worktree(worktree: &Worktree) -> Arc<dyn VcsDriver> {
    vcs::driver_for_kind(worktree.vcs_kind.clone())
}

pub async fn get_merge_queue_entry(
    state: &AppState,
    entry_id: MergeQueueEntryId,
) -> Result<MergeQueueEntry> {
    let workspaces = state.global_store().list_workspaces().await?;
    for workspace in workspaces {
        let store = state.store_for_workspace(workspace.id).await?;
        if let Some(entry) = store.get_merge_queue_entry(entry_id).await? {
            return Ok(entry);
        }
    }
    bail!("merge queue entry not found");
}

async fn list_queued_entries(state: &AppState) -> Result<Vec<MergeQueueEntry>> {
    let mut out = Vec::new();
    let workspaces = state.global_store().list_workspaces().await?;
    for workspace in workspaces {
        let store = state.store_for_workspace(workspace.id).await?;
        let mut entries = store.list_queued_merge_queue_entries().await?;
        out.append(&mut entries);
    }
    out.sort_by_key(|entry| entry.created_at);
    Ok(out)
}

pub async fn submit_merge_queue_entry(
    state: &Arc<AppState>,
    params: MergeQueueSubmitParams,
) -> Result<MergeQueueEntry> {
    let context = resolve_merge_queue_context(
        state,
        params.session_id,
        params.worktree_id,
        params.worktree_root,
    )
    .await?;
    let workspace = context.workspace;
    let mut worktree = context.worktree;
    let config = load_merge_queue_config(Path::new(&workspace.root_path)).await?;
    if !config.enabled {
        bail!("merge queue is disabled for this workspace");
    }

    let target_branch = params
        .target_branch
        .as_deref()
        .unwrap_or(&config.target_branch)
        .trim()
        .to_string();
    if target_branch.is_empty() {
        bail!("target_branch is required");
    }

    let vcs = context.vcs;
    let worktree_root = context.worktree_root;
    vcs.assert_repo(worktree_root.as_path()).await?;
    let dirty = vcs.status_porcelain(worktree_root.as_path()).await?;
    let dirty = if vcs.kind() == VcsKind::Jj {
        dirty
            .into_iter()
            .filter(|entry| entry.starts_with("?? "))
            .collect::<Vec<_>>()
    } else {
        dirty
    };
    if !dirty.is_empty() {
        bail!(
            "worktree has uncommitted changes:\n{}",
            dirty
                .iter()
                .take(24)
                .map(|entry| format!("- {}", entry))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    let merge_base = vcs
        .merge_base(worktree_root.as_path(), &target_branch, "HEAD")
        .await?;
    let worktree_patch = vcs
        .build_worktree_patch(worktree_root.as_path(), &merge_base)
        .await?;
    if worktree_patch.patch.trim().is_empty() {
        bail!("no changes detected; nothing to submit");
    }
    if worktree.is_none() {
        let worktree_id = WorktreeId::new();
        let worktree_record = Worktree {
            id: worktree_id,
            workspace_id: workspace.id,
            root_path: worktree_root.to_string_lossy().to_string(),
            base_commit_sha: worktree_patch.base_revision.clone(),
            git_branch: None,
            vcs_kind: Some(vcs.kind()),
            base_revision: Some(worktree_patch.base_revision.clone()),
            vcs_ref: None,
            created_at: Utc::now(),
            bootstrap_status: None,
            bootstrap_started_at: None,
            bootstrap_finished_at: None,
            bootstrap_exit_code: None,
            bootstrap_timeout_sec: None,
            bootstrap_error: None,
            bootstrap_log_path: None,
            bootstrap_log_truncated: None,
            bootstrap_config_path: None,
            bootstrap_config_key: None,
            bootstrap_command: None,
            bootstrap_script_path: None,
        };
        let store = state.store_for_workspace(workspace.id).await?;
        store.insert_worktree(worktree_record.clone()).await?;
        if let Err(err) = state
            .global_store()
            .upsert_workspace_worktree_index(worktree_id, workspace.id)
            .await
        {
            tracing::warn!(
                worktree_id = %worktree_id.0,
                "failed to update worktree index: {err:?}"
            );
        }
        worktree = Some(worktree_record);
    }
    let patch_source = MergeQueuePatchSource::Generated;
    let base_commit_sha = Some(worktree_patch.base_revision);
    let head_commit_sha = Some(worktree_patch.head_revision);
    let patch_text = worktree_patch.patch;

    let worktree = worktree
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("worktree is required to submit to the merge queue"))?;
    let entry_id = MergeQueueEntryId::new();
    let patch_path =
        write_patch_file(Path::new(&workspace.root_path), entry_id, &patch_text).await?;
    let now = Utc::now();
    let entry = MergeQueueEntry {
        id: entry_id,
        workspace_id: workspace.id,
        worktree_id: Some(worktree.id),
        session_id: params.session_id,
        target_branch,
        message: params.message,
        patch_source,
        base_commit_sha,
        head_commit_sha,
        patch_path: patch_path.to_string_lossy().to_string(),
        patch_size: patch_text.len() as i64,
        status: MergeQueueEntryStatus::Queued,
        result_commit_sha: None,
        error_message: None,
        created_at: now,
        updated_at: now,
    };
    let store = state.store_for_workspace(workspace.id).await?;
    store.create_merge_queue_entry(&entry).await?;
    state.transport.merge_queue_notify.notify_one();
    let entry = wait_for_merge_queue_completion(state, entry.id).await?;
    ensure_merge_queue_success(&entry)?;
    Ok(entry)
}

pub async fn cancel_merge_queue_entry(
    state: &Arc<AppState>,
    entry_id: MergeQueueEntryId,
) -> Result<MergeQueueEntry> {
    let mut entry = get_merge_queue_entry(state, entry_id).await?;
    let store = state.store_for_workspace(entry.workspace_id).await?;
    match entry.status {
        MergeQueueEntryStatus::Queued => {
            entry.status = MergeQueueEntryStatus::Cancelled;
            entry.updated_at = Utc::now();
            store.update_merge_queue_entry(&entry).await?;
            state.transport.merge_queue_notify.notify_waiters();
            Ok(entry)
        }
        MergeQueueEntryStatus::Running => {
            bail!("cannot cancel a running merge queue entry");
        }
        _ => Ok(entry),
    }
}

pub async fn retry_merge_queue_entry(
    state: &Arc<AppState>,
    entry_id: MergeQueueEntryId,
) -> Result<MergeQueueEntry> {
    let mut entry = get_merge_queue_entry(state, entry_id).await?;
    let store = state.store_for_workspace(entry.workspace_id).await?;
    match entry.status {
        MergeQueueEntryStatus::Failed | MergeQueueEntryStatus::Conflict => {
            entry.status = MergeQueueEntryStatus::Queued;
            entry.error_message = None;
            entry.result_commit_sha = None;
            entry.updated_at = Utc::now();
            store.update_merge_queue_entry(&entry).await?;
            state.transport.merge_queue_notify.notify_one();
            state.transport.merge_queue_notify.notify_waiters();
            Ok(entry)
        }
        _ => Ok(entry),
    }
}

pub fn spawn_merge_queue_runner(state: Arc<AppState>) {
    let notify = state.transport.merge_queue_notify.clone();
    tokio::spawn(async move {
        loop {
            match run_next_entry(&state).await {
                Ok(true) => continue,
                Ok(false) => {}
                Err(err) => {
                    tracing::warn!("merge queue runner error: {err:#}");
                }
            }
            notify.notified().await;
        }
    });
}

async fn run_next_entry(state: &Arc<AppState>) -> Result<bool> {
    let entries = list_queued_entries(state).await?;
    for mut entry in entries {
        let workspace = match state
            .global_store()
            .get_workspace(entry.workspace_id)
            .await?
        {
            Some(ws) => ws,
            None => continue,
        };
        let store = state.store_for_workspace(workspace.id).await?;
        let cfg = load_merge_queue_config(Path::new(&workspace.root_path)).await?;
        if !cfg.enabled {
            continue;
        }
        let now = Utc::now();
        let claimed = store.claim_merge_queue_entry(entry.id, now).await?;
        if !claimed {
            continue;
        }
        entry.status = MergeQueueEntryStatus::Running;
        entry.updated_at = now;
        run_entry(state, &workspace, entry, &cfg).await?;
        return Ok(true);
    }
    Ok(false)
}

async fn run_entry(
    state: &Arc<AppState>,
    workspace: &Workspace,
    mut entry: MergeQueueEntry,
    cfg: &MergeQueueConfig,
) -> Result<()> {
    let store = state.store_for_workspace(workspace.id).await?;
    let run_id = MergeQueueRunId::new();
    let log_path = merge_queue_log_path(Path::new(&workspace.root_path), run_id);
    let mut run = MergeQueueRun {
        id: run_id,
        entry_id: entry.id,
        status: MergeQueueRunStatus::Running,
        started_at: Utc::now(),
        finished_at: None,
        exit_code: None,
        log_path: Some(log_path.to_string_lossy().to_string()),
        error_message: None,
        result_commit_sha: None,
    };
    store.create_merge_queue_run(&run).await?;

    let mut log_file = open_log_file(&log_path).await?;
    write_log_line(&mut log_file, "# ctx merge queue\n").await?;
    write_log_line(
        &mut log_file,
        &format!("entry: {} target: {}\n", entry.id.0, entry.target_branch),
    )
    .await?;

    let result = run_entry_inner(state, workspace, &entry, cfg, &mut log_file).await;
    let now = Utc::now();
    match result {
        Ok(commit_sha) => {
            entry.status = MergeQueueEntryStatus::Passed;
            entry.result_commit_sha = Some(commit_sha.clone());
            entry.error_message = None;
            entry.updated_at = now;
            run.status = MergeQueueRunStatus::Passed;
            run.result_commit_sha = Some(commit_sha.clone());
            run.finished_at = Some(now);
            store.update_merge_queue_entry(&entry).await?;
            store.update_merge_queue_run(&run).await?;
            state.transport.merge_queue_notify.notify_waiters();
            if let Err(err) =
                maybe_sync_originating_worktree(state, workspace, &entry, &commit_sha).await
            {
                tracing::warn!("merge queue sync failed: {err:#}");
            }
        }
        Err(QueueError::Conflict { message }) => {
            entry.status = MergeQueueEntryStatus::Conflict;
            entry.error_message = Some(message.clone());
            entry.updated_at = now;
            run.status = MergeQueueRunStatus::Conflict;
            run.error_message = Some(message);
            run.finished_at = Some(now);
            store.update_merge_queue_entry(&entry).await?;
            store.update_merge_queue_run(&run).await?;
            state.transport.merge_queue_notify.notify_waiters();
        }
        Err(QueueError::Failed {
            message,
            exit_code,
            result_commit_sha,
        }) => {
            entry.status = MergeQueueEntryStatus::Failed;
            entry.error_message = Some(message.clone());
            entry.result_commit_sha = result_commit_sha.clone();
            entry.updated_at = now;
            run.status = MergeQueueRunStatus::Failed;
            run.exit_code = exit_code;
            run.error_message = Some(message);
            run.result_commit_sha = result_commit_sha;
            run.finished_at = Some(now);
            store.update_merge_queue_entry(&entry).await?;
            store.update_merge_queue_run(&run).await?;
            state.transport.merge_queue_notify.notify_waiters();
        }
    }

    Ok(())
}

async fn wait_for_merge_queue_completion(
    state: &Arc<AppState>,
    entry_id: MergeQueueEntryId,
) -> Result<MergeQueueEntry> {
    let entry = get_merge_queue_entry(state, entry_id).await?;
    let store = state.store_for_workspace(entry.workspace_id).await?;
    let notify = state.transport.merge_queue_notify.clone();
    loop {
        let entry = store
            .get_merge_queue_entry(entry_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("merge queue entry not found"))?;
        match entry.status {
            MergeQueueEntryStatus::Queued | MergeQueueEntryStatus::Running => {
                let notified = notify.notified();
                tokio::select! {
                    _ = notified => {}
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                }
            }
            _ => return Ok(entry),
        }
    }
}

fn ensure_merge_queue_success(entry: &MergeQueueEntry) -> Result<()> {
    match entry.status {
        MergeQueueEntryStatus::Passed => Ok(()),
        MergeQueueEntryStatus::Conflict => bail!(
            "merge queue conflict for entry {}: {}",
            entry.id.0,
            entry
                .error_message
                .as_deref()
                .unwrap_or("conflict while applying changes")
        ),
        MergeQueueEntryStatus::Failed => bail!(
            "merge queue failed for entry {}: {}",
            entry.id.0,
            entry
                .error_message
                .as_deref()
                .unwrap_or("merge queue run failed")
        ),
        MergeQueueEntryStatus::Cancelled => bail!("merge queue entry {} was cancelled", entry.id.0),
        MergeQueueEntryStatus::Queued | MergeQueueEntryStatus::Running => bail!(
            "merge queue entry {} is still running; try again",
            entry.id.0
        ),
    }
}

async fn run_entry_inner(
    state: &Arc<AppState>,
    workspace: &Workspace,
    entry: &MergeQueueEntry,
    cfg: &MergeQueueConfig,
    log_file: &mut fs::File,
) -> std::result::Result<String, QueueError> {
    let vcs = vcs::driver_for_path(Path::new(&workspace.root_path))
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
    vcs.assert_repo(Path::new(&workspace.root_path))
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;

    let worktree_path =
        merge_queue_worktree_path(Path::new(&workspace.root_path), workspace.id, entry.id);
    let worktree_branch = if vcs.kind() == VcsKind::Jj {
        format!("ctx-merge-queue-{}", entry.id.0)
    } else {
        format!("ctx-merge-queue/{}", entry.id.0)
    };
    let (repo_root, target_head) = if vcs.kind() == VcsKind::Git {
        let repo_root = ensure_merge_queue_repo(state, entry, workspace, cfg, log_file).await?;
        let target_head =
            ensure_merge_queue_target_branch(state, entry, &repo_root, &entry.target_branch)
                .await?;
        let _ = remove_worktree(&repo_root, &worktree_path).await;
        let _ = delete_branch(&repo_root, &worktree_branch).await;
        create_worktree(&repo_root, &worktree_path, &target_head, &worktree_branch)
            .await
            .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
        (Some(repo_root), target_head)
    } else {
        let target_head =
            resolve_target_head(vcs.as_ref(), &workspace.root_path, &entry.target_branch)
                .await
                .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
        let _ = remove_worktree(&workspace.root_path, &worktree_path).await;
        create_worktree(
            &workspace.root_path,
            &worktree_path,
            &target_head,
            &worktree_branch,
        )
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
        (None, target_head)
    };
    let git_repo_root = repo_root
        .as_deref()
        .unwrap_or_else(|| Path::new(&workspace.root_path));

    if vcs.kind() == VcsKind::Jj {
        ensure_jj_working_copy(&worktree_path, &target_head, log_file, vcs.as_ref()).await?;
    }

    let result = async {
        let patch = read_patch_file(&entry.patch_path)
            .await
            .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
        write_log_line(log_file, "apply patch\n")
            .await
            .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
        let apply_target = if vcs.kind() == VcsKind::Git {
            ApplyPatchTarget::Index
        } else {
            ApplyPatchTarget::Worktree
        };
        if let Err(err) = apply_patch(
            state,
            entry,
            vcs.as_ref(),
            git_repo_root,
            &worktree_path,
            &patch,
            apply_target,
        )
        .await
        {
            match err {
                QueueError::Conflict { message } => {
                    let _ = write_log_line(
                        log_file,
                        &format!("apply patch conflict: {message}\n"),
                    )
                    .await;
                    return Err(QueueError::Conflict {
                        message: MERGE_QUEUE_CONFLICT_MESSAGE.to_string(),
                    });
                }
                other => return Err(other),
            }
        }

        let has_changes = if vcs.kind() == VcsKind::Git {
            has_staged_changes(state, entry, &worktree_path).await?
        } else {
            has_worktree_changes(vcs.as_ref(), &worktree_path, &target_head).await?
        };
        if !has_changes {
            return Err(QueueError::fail(
                "patch did not produce any changes".to_string(),
                None,
                None,
            ));
        }

        let message = entry
            .message
            .as_deref()
            .filter(|m| !m.trim().is_empty())
            .unwrap_or("merge queue entry");
        commit_changes(state, entry, &worktree_path, vcs.kind(), message, log_file).await?;
        let commit_sha = vcs
            .rev_parse_head(&worktree_path)
            .await
            .map_err(|e| QueueError::fail(e.to_string(), None, None))?;

        for cmd in &cfg.verify_commands {
            run_verify_command(state, &worktree_path, entry, cmd, log_file).await?;
        }

        let target_checkout = if vcs.kind() == VcsKind::Git {
            find_checked_out_worktree_for_branch(
                state,
                entry,
                git_repo_root,
                &entry.target_branch,
            )
            .await
            .map_err(|e| QueueError::fail(e.to_string(), None, Some(commit_sha.clone())))?
        } else {
            None
        };
        if let Some(path) = target_checkout.as_ref() {
            let dirty = vcs
                .status_porcelain(Path::new(path))
                .await
                .map_err(|e| QueueError::fail(e.to_string(), None, Some(commit_sha.clone())))?;
            if !dirty.is_empty() {
                return Err(QueueError::fail(
                    format!(
                        "target branch {} is checked out at {} with uncommitted changes",
                        entry.target_branch, path
                    ),
                    None,
                    Some(commit_sha.clone()),
                ));
            }
        }

        write_log_line(
            log_file,
            &format!("advance target branch {}\n", entry.target_branch),
        )
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
        if let Some(path) = target_checkout.as_ref() {
            let previous_head = vcs
                .rev_parse_ref(Path::new(path), "HEAD")
                .await
                .map_err(|e| QueueError::fail(e.to_string(), None, Some(commit_sha.clone())))?;
            if previous_head != target_head {
                return Err(QueueError::fail(
                    format!(
                        "failed to update target branch: expected {target_head}, found {previous_head}"
                    ),
                    None,
                    Some(commit_sha.clone()),
                ));
            }
            reset_worktree_to_commit(state, entry, path, &commit_sha)
                .await
                .map_err(|e| QueueError::fail(e.to_string(), None, Some(commit_sha.clone())))?;
            let _ = maybe_update_worktree_base_commit_for_path(
                state,
                workspace.id,
                path,
                &commit_sha,
            )
            .await
            .map_err(|e| QueueError::fail(e.to_string(), None, Some(commit_sha.clone())))?;
        } else {
            update_target_branch(
                state,
                entry,
                git_repo_root,
                Path::new(&workspace.root_path),
                vcs.kind(),
                &entry.target_branch,
                &commit_sha,
                &target_head,
            )
            .await?;
        }

        if cfg.push_on_success {
            write_log_line(
                log_file,
                &format!(
                    "push {} {}:{}\n",
                    cfg.push_remote, entry.target_branch, cfg.push_branch
                ),
            )
            .await
            .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
            if let Err(err) = push_target_branch(
                state,
                entry,
                git_repo_root,
                Path::new(&workspace.root_path),
                vcs.kind(),
                &cfg.push_remote,
                &entry.target_branch,
                &cfg.push_branch,
                &commit_sha,
            )
            .await
            {
                return Err(QueueError::fail(
                    format!("push_failed: {err}"),
                    None,
                    Some(commit_sha),
                ));
            }
        }

        if vcs.kind() == VcsKind::Git {
            if let Some(repo_root) = repo_root.as_ref() {
                if let Err(err) = maybe_sync_canonical_worktree(
                    state,
                    workspace,
                    entry,
                    repo_root,
                    &commit_sha,
                    cfg.canonical_sync,
                    log_file,
                )
                .await
                {
                    let _ = write_log_line(
                        log_file,
                        &format!("canonical sync failed: {err:#}\n"),
                    )
                    .await;
                    tracing::warn!("merge queue canonical sync failed: {err:#}");
                }
            }
        }

        Ok(commit_sha)
    }
    .await;

    if vcs.kind() == VcsKind::Git {
        if let Some(repo_root) = repo_root.as_ref() {
            let _ = remove_worktree(repo_root, &worktree_path).await;
            let _ = delete_branch(repo_root, &worktree_branch).await;
        }
    } else {
        let _ = remove_worktree(&workspace.root_path, &worktree_path).await;
    }
    result
}

struct MergeQueueWorktreeContext {
    workspace: Workspace,
    worktree: Option<Worktree>,
    worktree_root: PathBuf,
    vcs: Arc<dyn VcsDriver>,
}

async fn resolve_merge_queue_context(
    state: &Arc<AppState>,
    session_id: Option<SessionId>,
    worktree_id: Option<WorktreeId>,
    worktree_root: Option<String>,
) -> Result<MergeQueueWorktreeContext> {
    if let Some(worktree_id) = worktree_id {
        let store = state.store_for_worktree(worktree_id).await?;
        let worktree = store
            .get_worktree(worktree_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("worktree not found"))?;
        let workspace = state
            .global_store()
            .get_workspace(worktree.workspace_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("workspace not found"))?;
        let vcs = vcs_driver_for_worktree(&worktree);
        let worktree_root = PathBuf::from(&worktree.root_path);
        return Ok(MergeQueueWorktreeContext {
            workspace,
            worktree: Some(worktree),
            worktree_root,
            vcs,
        });
    }

    let session_id = session_id.ok_or_else(|| anyhow::anyhow!("session_id is required"))?;
    let store = state.store_for_session(session_id).await?;
    let session = store
        .get_session(session_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("session not found"))?;
    let workspace = state
        .global_store()
        .get_workspace(session.workspace_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("workspace not found"))?;

    if let Some(root) = worktree_root {
        let root_path = PathBuf::from(root.trim());
        if !root_path.is_absolute() {
            bail!("worktree_root must be an absolute path");
        }
        let root_string = root_path.to_string_lossy().to_string();
        if let Some(worktree) = store
            .get_worktree_for_root(workspace.id, &root_string)
            .await?
        {
            let vcs = vcs_driver_for_worktree(&worktree);
            let worktree_root = PathBuf::from(&worktree.root_path);
            return Ok(MergeQueueWorktreeContext {
                workspace,
                worktree: Some(worktree),
                worktree_root,
                vcs,
            });
        }
        let vcs = vcs::driver_for_path(&root_path).await?;
        return Ok(MergeQueueWorktreeContext {
            workspace,
            worktree: None,
            worktree_root: root_path,
            vcs,
        });
    }

    let worktree = store
        .get_worktree(session.worktree_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("worktree not found"))?;
    let vcs = vcs_driver_for_worktree(&worktree);
    let worktree_root = PathBuf::from(&worktree.root_path);
    Ok(MergeQueueWorktreeContext {
        workspace,
        worktree: Some(worktree),
        worktree_root,
        vcs,
    })
}

fn merge_queue_root(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".ctx").join("merge-queue")
}

fn merge_queue_repo_root(workspace_root: &Path) -> PathBuf {
    merge_queue_root(workspace_root).join("repo")
}

fn merge_queue_patch_path(workspace_root: &Path, entry_id: MergeQueueEntryId) -> PathBuf {
    merge_queue_root(workspace_root)
        .join("patches")
        .join(format!("{}.patch", entry_id.0))
}

fn merge_queue_log_path(workspace_root: &Path, run_id: MergeQueueRunId) -> PathBuf {
    merge_queue_root(workspace_root)
        .join("logs")
        .join(format!("{}.log", run_id.0))
}

fn merge_queue_worktree_path(
    workspace_root: &Path,
    workspace_id: WorkspaceId,
    entry_id: MergeQueueEntryId,
) -> PathBuf {
    merge_queue_root(workspace_root)
        .join("worktrees")
        .join(workspace_id.0.to_string())
        .join(entry_id.0.to_string())
}

async fn write_patch_file(
    workspace_root: &Path,
    entry_id: MergeQueueEntryId,
    patch: &str,
) -> Result<PathBuf> {
    let path = merge_queue_patch_path(workspace_root, entry_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .context("creating merge queue patch dir")?;
    }
    fs::write(&path, patch)
        .await
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

async fn read_patch_file(path: &str) -> Result<String> {
    let data = fs::read_to_string(path)
        .await
        .with_context(|| format!("reading {}", path))?;
    Ok(data)
}

async fn open_log_file(path: &Path) -> Result<fs::File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .context("creating merge queue log dir")?;
    }
    let file = fs::File::create(path)
        .await
        .with_context(|| format!("opening {}", path.display()))?;
    Ok(file)
}

async fn write_log_line(file: &mut fs::File, line: &str) -> Result<()> {
    file.write_all(line.as_bytes()).await?;
    Ok(())
}

async fn ensure_jj_working_copy(
    worktree_path: &Path,
    target_head: &str,
    log_file: &mut fs::File,
    vcs: &dyn VcsDriver,
) -> std::result::Result<(), QueueError> {
    let current = vcs
        .rev_parse_head(worktree_path)
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
    if current.trim() != target_head.trim() {
        return Ok(());
    }
    write_log_line(log_file, "jj new\n")
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
    let output = vcs::jj_command_output(worktree_path, &["new"])
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
    write_log_line(log_file, &String::from_utf8_lossy(&output.stdout))
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
    write_log_line(log_file, &String::from_utf8_lossy(&output.stderr))
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
    if !output.status.success() {
        return Err(QueueError::fail(
            "jj new failed".to_string(),
            Some(output.status.code().unwrap_or(1) as i64),
            None,
        ));
    }
    Ok(())
}

async fn ensure_merge_queue_repo(
    state: &AppState,
    entry: &MergeQueueEntry,
    workspace: &Workspace,
    cfg: &MergeQueueConfig,
    log_file: &mut fs::File,
) -> std::result::Result<PathBuf, QueueError> {
    let workspace_root = Path::new(&workspace.root_path);
    let repo_root = merge_queue_repo_root(workspace_root);
    if let Some(parent) = repo_root.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
    }

    let needs_init = match fs::metadata(&repo_root).await {
        Ok(meta) => !meta.is_dir() || !repo_root.join(".git").exists(),
        Err(_) => true,
    };
    if needs_init {
        fs::create_dir_all(&repo_root)
            .await
            .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
        write_log_line(log_file, "init merge queue repo\n")
            .await
            .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
        let mut cmd =
            merge_queue_command(state, entry, "git init", "git", Some(&repo_root), &[]).await;
        let output = cmd
            .arg("-C")
            .arg(&repo_root)
            .arg("init")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(QueueError::fail(
                format!("merge queue repo init failed: {stderr}"),
                Some(output.status.code().unwrap_or(1) as i64),
                None,
            ));
        }
        let mut cmd = merge_queue_command(
            state,
            entry,
            "git symbolic-ref HEAD",
            "git",
            Some(&repo_root),
            &[],
        )
        .await;
        let output = cmd
            .arg("-C")
            .arg(&repo_root)
            .args(["symbolic-ref", "HEAD", MERGE_QUEUE_HEAD_REF])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(QueueError::fail(
                format!("merge queue repo HEAD update failed: {stderr}"),
                Some(output.status.code().unwrap_or(1) as i64),
                None,
            ));
        }
    }

    ensure_git_remote(
        state,
        entry,
        &repo_root,
        MERGE_QUEUE_CANONICAL_REMOTE,
        workspace.root_path.as_str(),
    )
    .await?;

    if cfg.push_remote != MERGE_QUEUE_CANONICAL_REMOTE {
        if let Some(url) =
            git_remote_get_url(state, entry, workspace_root, &cfg.push_remote).await?
        {
            ensure_git_remote(state, entry, &repo_root, &cfg.push_remote, url.trim()).await?;
        } else if cfg.push_on_success {
            return Err(QueueError::fail(
                format!(
                    "merge queue push remote {} not configured in canonical repo",
                    cfg.push_remote
                ),
                None,
                None,
            ));
        }
    }

    Ok(repo_root)
}

async fn ensure_merge_queue_target_branch(
    state: &AppState,
    entry: &MergeQueueEntry,
    repo_root: &Path,
    target_branch: &str,
) -> std::result::Result<String, QueueError> {
    if merge_queue_branch_exists(state, entry, repo_root, target_branch).await? {
        return rev_parse_ref(repo_root, target_branch)
            .await
            .map_err(|e| QueueError::fail(e.to_string(), None, None));
    }

    let mut cmd = merge_queue_command(
        state,
        entry,
        "git fetch canonical",
        "git",
        Some(repo_root),
        &[],
    )
    .await;
    let refspec = format!("refs/heads/{target_branch}:refs/heads/{target_branch}");
    let output = cmd
        .arg("-C")
        .arg(repo_root)
        .args(["fetch", MERGE_QUEUE_CANONICAL_REMOTE, &refspec])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(QueueError::fail(
            format!("merge queue fetch failed: {stderr}"),
            Some(output.status.code().unwrap_or(1) as i64),
            None,
        ));
    }

    rev_parse_ref(repo_root, target_branch)
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))
}

async fn merge_queue_branch_exists(
    state: &AppState,
    entry: &MergeQueueEntry,
    repo_root: &Path,
    target_branch: &str,
) -> std::result::Result<bool, QueueError> {
    let mut cmd = merge_queue_command(
        state,
        entry,
        "git show-ref --verify",
        "git",
        Some(repo_root),
        &[],
    )
    .await;
    let output = cmd
        .arg("-C")
        .arg(repo_root)
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{target_branch}"),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
    if output.status.success() {
        return Ok(true);
    }
    if output.status.code() == Some(1) {
        return Ok(false);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(QueueError::fail(
        format!("git show-ref failed: {stderr}"),
        Some(output.status.code().unwrap_or(1) as i64),
        None,
    ))
}

async fn git_remote_get_url(
    state: &AppState,
    entry: &MergeQueueEntry,
    repo_root: &Path,
    remote: &str,
) -> std::result::Result<Option<String>, QueueError> {
    let mut cmd = merge_queue_command(
        state,
        entry,
        "git remote get-url",
        "git",
        Some(repo_root),
        &[],
    )
    .await;
    let output = cmd
        .arg("-C")
        .arg(repo_root)
        .args(["remote", "get-url", remote])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
    if output.status.success() {
        return Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
        ));
    }
    if output.status.code() == Some(2) {
        return Ok(None);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(QueueError::fail(
        format!("git remote get-url failed: {stderr}"),
        Some(output.status.code().unwrap_or(1) as i64),
        None,
    ))
}

async fn ensure_git_remote(
    state: &AppState,
    entry: &MergeQueueEntry,
    repo_root: &Path,
    remote: &str,
    url: &str,
) -> std::result::Result<(), QueueError> {
    if let Some(existing) = git_remote_get_url(state, entry, repo_root, remote).await? {
        if existing.trim() == url {
            return Ok(());
        }
        let mut cmd = merge_queue_command(
            state,
            entry,
            "git remote set-url",
            "git",
            Some(repo_root),
            &[],
        )
        .await;
        let output = cmd
            .arg("-C")
            .arg(repo_root)
            .args(["remote", "set-url", remote, url])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(QueueError::fail(
                format!("git remote set-url failed: {stderr}"),
                Some(output.status.code().unwrap_or(1) as i64),
                None,
            ));
        }
        return Ok(());
    }

    let mut cmd =
        merge_queue_command(state, entry, "git remote add", "git", Some(repo_root), &[]).await;
    let output = cmd
        .arg("-C")
        .arg(repo_root)
        .args(["remote", "add", remote, url])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(QueueError::fail(
            format!("git remote add failed: {stderr}"),
            Some(output.status.code().unwrap_or(1) as i64),
            None,
        ));
    }
    Ok(())
}

async fn maybe_update_worktree_base_commit_for_path(
    state: &AppState,
    workspace_id: WorkspaceId,
    worktree_path: &str,
    commit_sha: &str,
) -> Result<Option<Worktree>> {
    let store = state.store_for_workspace(workspace_id).await?;
    let worktrees = store.list_worktrees(workspace_id).await?;
    let checkout_path = fs::canonicalize(worktree_path)
        .await
        .unwrap_or_else(|_| PathBuf::from(worktree_path));
    for worktree in worktrees {
        let root_path = fs::canonicalize(&worktree.root_path)
            .await
            .unwrap_or_else(|_| PathBuf::from(&worktree.root_path));
        if root_path == checkout_path {
            store
                .update_worktree_base_commit(worktree.id, commit_sha)
                .await?;
            return Ok(Some(worktree));
        }
    }
    Ok(None)
}

async fn maybe_sync_canonical_worktree(
    state: &Arc<AppState>,
    workspace: &Workspace,
    entry: &MergeQueueEntry,
    merge_queue_repo_root: &Path,
    commit_sha: &str,
    policy: MergeQueueCanonicalSync,
    log_file: &mut fs::File,
) -> Result<()> {
    if policy == MergeQueueCanonicalSync::Never {
        write_log_line(log_file, "canonical sync disabled\n").await?;
        return Ok(());
    }

    let target_checkout = find_checked_out_worktree_for_branch(
        state,
        entry,
        Path::new(&workspace.root_path),
        &entry.target_branch,
    )
    .await?;
    let Some(path) = target_checkout else {
        write_log_line(
            log_file,
            "canonical sync skipped: target branch not checked out\n",
        )
        .await?;
        return Ok(());
    };
    let canonical_root = fs::canonicalize(&workspace.root_path)
        .await
        .unwrap_or_else(|_| PathBuf::from(&workspace.root_path));
    let checkout_root = fs::canonicalize(&path)
        .await
        .unwrap_or_else(|_| PathBuf::from(&path));
    if checkout_root != canonical_root {
        write_log_line(
            log_file,
            "canonical sync skipped: target branch checked out in another worktree\n",
        )
        .await?;
        return Ok(());
    }

    let dirty = git_status_porcelain(&path).await?;
    if !dirty.is_empty() && policy == MergeQueueCanonicalSync::CleanOnly {
        let message = format!(
            "canonical sync skipped: target branch {} is dirty at {}",
            entry.target_branch, path
        );
        write_log_line(log_file, &format!("{message}\n")).await?;
        if let Some(session_id) = entry.session_id {
            emit_merge_queue_canonical_sync_notice(
                state,
                session_id,
                None,
                &entry.target_branch,
                commit_sha,
                "skipped",
                &message,
            )
            .await?;
        }
        return Ok(());
    }

    write_log_line(
        log_file,
        &format!(
            "canonical sync: fetch {} from merge-queue repo\n",
            entry.target_branch
        ),
    )
    .await?;
    fetch_merge_queue_target_branch(
        state,
        entry,
        Path::new(&workspace.root_path),
        merge_queue_repo_root,
        &entry.target_branch,
    )
    .await?;
    let previous_head = rev_parse_ref(&path, "HEAD")
        .await
        .unwrap_or_else(|_| "unknown".to_string());
    write_log_line(
        log_file,
        &format!(
            "canonical sync: reset {} to {}\n",
            entry.target_branch, commit_sha
        ),
    )
    .await?;
    reset_worktree_to_commit(state, entry, &path, commit_sha).await?;
    let worktree =
        maybe_update_worktree_base_commit_for_path(state, workspace.id, &path, commit_sha).await?;
    if let Some(session_id) = entry.session_id {
        if let Some(worktree) = worktree {
            emit_merge_queue_sync_notice(
                state,
                session_id,
                &worktree,
                &entry.target_branch,
                &previous_head,
                commit_sha,
            )
            .await?;
        } else {
            emit_merge_queue_canonical_sync_notice(
                state,
                session_id,
                None,
                &entry.target_branch,
                commit_sha,
                "applied",
                &format!(
                    "canonical sync applied; reset {path} from {previous_head} to {commit_sha}",
                ),
            )
            .await?;
        }
    }
    Ok(())
}

async fn fetch_merge_queue_target_branch(
    state: &AppState,
    entry: &MergeQueueEntry,
    canonical_root: &Path,
    merge_queue_repo_root: &Path,
    target_branch: &str,
) -> Result<()> {
    let mut cmd = merge_queue_command(
        state,
        entry,
        "git fetch merge-queue repo",
        "git",
        Some(canonical_root),
        &[],
    )
    .await;
    let output = cmd
        .arg("-C")
        .arg(canonical_root)
        .args([
            "fetch",
            merge_queue_repo_root
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("invalid merge queue repo path"))?,
            &format!("refs/heads/{target_branch}"),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git fetch")?;
    if !output.status.success() {
        bail!(
            "git fetch from merge queue repo failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

async fn apply_patch(
    state: &AppState,
    entry: &MergeQueueEntry,
    vcs: &dyn VcsDriver,
    repo_root: &Path,
    worktree_path: &Path,
    patch: &str,
    target: ApplyPatchTarget,
) -> std::result::Result<(), QueueError> {
    if vcs.kind() == VcsKind::Git {
        let output = run_git_apply(
            state,
            entry,
            worktree_path,
            patch,
            matches!(target, ApplyPatchTarget::Index),
            true,
        )
        .await?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if matches!(target, ApplyPatchTarget::Index) && stderr.contains("does not exist in index") {
            let output = run_git_apply(state, entry, worktree_path, patch, false, false).await?;
            if output.status.success() {
                stage_worktree(state, entry, worktree_path).await?;
                return Ok(());
            }
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(QueueError::Conflict {
                message: if stderr.is_empty() {
                    "patch conflict".to_string()
                } else {
                    stderr
                },
            });
        }
        return Err(QueueError::Conflict {
            message: if stderr.is_empty() {
                "patch conflict".to_string()
            } else {
                stderr
            },
        });
    }

    if vcs.kind() == VcsKind::Jj {
        let rel = worktree_path.strip_prefix(repo_root).map_err(|_| {
            QueueError::fail("jj worktree is outside repo root".to_string(), None, None)
        })?;
        let mut cmd =
            merge_queue_command(state, entry, "git apply", "git", Some(repo_root), &[]).await;
        cmd.arg("-C")
            .arg(repo_root)
            .arg("apply")
            .arg("--whitespace=nowarn")
            .arg("--directory")
            .arg(rel)
            .arg("-");
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| QueueError::fail(e.to_string(), None, None))?;

        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin
                .write_all(patch.as_bytes())
                .await
                .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
        }

        let output = child
            .wait_with_output()
            .await
            .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(QueueError::Conflict {
                message: if stderr.is_empty() {
                    "patch conflict".to_string()
                } else {
                    stderr
                },
            });
        }
        return Ok(());
    }

    vcs.apply_patch(worktree_path, patch, target, false)
        .await
        .map_err(|e| QueueError::Conflict {
            message: e.to_string(),
        })?;
    Ok(())
}

async fn run_git_apply(
    state: &AppState,
    entry: &MergeQueueEntry,
    worktree_path: &Path,
    patch: &str,
    apply_index: bool,
    three_way: bool,
) -> std::result::Result<std::process::Output, QueueError> {
    let mut cmd =
        merge_queue_command(state, entry, "git apply", "git", Some(worktree_path), &[]).await;
    cmd.arg("-C")
        .arg(worktree_path)
        .arg("apply")
        .arg("--whitespace=nowarn");
    if three_way {
        cmd.arg("--3way");
    }
    if apply_index {
        cmd.arg("--index");
    }
    cmd.arg("-");
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;

    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin
            .write_all(patch.as_bytes())
            .await
            .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
    }

    child
        .wait_with_output()
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))
}

async fn stage_worktree(
    state: &AppState,
    entry: &MergeQueueEntry,
    worktree_path: &Path,
) -> std::result::Result<(), QueueError> {
    let mut cmd =
        merge_queue_command(state, entry, "git add -A", "git", Some(worktree_path), &[]).await;
    let output = cmd
        .arg("-C")
        .arg(worktree_path)
        .args(["add", "-A"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(QueueError::fail(
            format!("git add failed: {stderr}"),
            Some(output.status.code().unwrap_or(1) as i64),
            None,
        ));
    }
    Ok(())
}

async fn has_staged_changes(
    state: &AppState,
    entry: &MergeQueueEntry,
    worktree_path: &Path,
) -> std::result::Result<bool, QueueError> {
    let mut cmd = merge_queue_command(
        state,
        entry,
        "git diff --cached --quiet",
        "git",
        Some(worktree_path),
        &[],
    )
    .await;
    let status = cmd
        .arg("-C")
        .arg(worktree_path)
        .args(["diff", "--cached", "--quiet"])
        .status()
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
    Ok(!status.success())
}

async fn has_worktree_changes(
    vcs: &dyn VcsDriver,
    worktree_path: &Path,
    base_revision: &str,
) -> std::result::Result<bool, QueueError> {
    let diff = vcs
        .diff(worktree_path, base_revision)
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
    Ok(!diff.trim().is_empty())
}

async fn commit_changes(
    state: &AppState,
    entry: &MergeQueueEntry,
    worktree_path: &Path,
    vcs_kind: VcsKind,
    message: &str,
    log_file: &mut fs::File,
) -> std::result::Result<(), QueueError> {
    match vcs_kind {
        VcsKind::Git => {
            let mut cmd =
                merge_queue_command(state, entry, "git commit", "git", Some(worktree_path), &[])
                    .await;
            let output = cmd
                .arg("-C")
                .arg(worktree_path)
                .args([
                    "-c",
                    "user.name=ctx",
                    "-c",
                    "user.email=ctx@local",
                    "-c",
                    "commit.gpgsign=false",
                    "commit",
                    "-m",
                    message,
                ])
                .output()
                .await
                .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
            write_log_line(log_file, &String::from_utf8_lossy(&output.stdout))
                .await
                .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
            write_log_line(log_file, &String::from_utf8_lossy(&output.stderr))
                .await
                .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
            if !output.status.success() {
                return Err(QueueError::fail(
                    "git commit failed".to_string(),
                    Some(output.status.code().unwrap_or(1) as i64),
                    None,
                ));
            }
            Ok(())
        }
        VcsKind::Jj => {
            let mut cmd =
                merge_queue_command(state, entry, "jj describe", "jj", Some(worktree_path), &[])
                    .await;
            let output = cmd
                .arg("-R")
                .arg(worktree_path)
                .arg("--color=never")
                .arg("--no-pager")
                .args(["describe", "-m", message])
                .output()
                .await
                .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
            write_log_line(log_file, &String::from_utf8_lossy(&output.stdout))
                .await
                .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
            write_log_line(log_file, &String::from_utf8_lossy(&output.stderr))
                .await
                .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
            if !output.status.success() {
                return Err(QueueError::fail(
                    "jj describe failed".to_string(),
                    Some(output.status.code().unwrap_or(1) as i64),
                    None,
                ));
            }
            Ok(())
        }
        VcsKind::Hg | VcsKind::Svn | VcsKind::P4 | VcsKind::Other => Err(QueueError::fail(
            format!("merge queue does not support {vcs_kind:?} commits"),
            None,
            None,
        )),
    }
}

async fn run_verify_command(
    state: &AppState,
    worktree_path: &Path,
    entry: &MergeQueueEntry,
    command: &str,
    log_file: &mut fs::File,
) -> std::result::Result<(), QueueError> {
    write_log_line(log_file, &format!("verify: {command}\n"))
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
    let envs = vec![
        (
            "CTX_MERGE_QUEUE_ENTRY_ID".to_string(),
            entry.id.0.to_string(),
        ),
        (
            "CTX_WORKTREE_ROOT".to_string(),
            worktree_path.to_string_lossy().to_string(),
        ),
        ("CTX_TARGET_BRANCH".to_string(), entry.target_branch.clone()),
    ];
    let mut cmd = command_for_shell(state, entry, command, worktree_path, &envs).await;
    cmd.stdin(Stdio::null());
    let output = cmd
        .output()
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
    write_log_line(log_file, &String::from_utf8_lossy(&output.stdout))
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
    write_log_line(log_file, &String::from_utf8_lossy(&output.stderr))
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
    if !output.status.success() {
        return Err(QueueError::fail(
            format!("verify failed: {command}"),
            Some(output.status.code().unwrap_or(1) as i64),
            None,
        ));
    }
    Ok(())
}

async fn maybe_sync_originating_worktree(
    state: &Arc<AppState>,
    workspace: &Workspace,
    entry: &MergeQueueEntry,
    commit_sha: &str,
) -> Result<()> {
    let Some(worktree_id) = entry.worktree_id else {
        return Ok(());
    };
    let store = state.store_for_worktree(worktree_id).await?;
    let Some(worktree) = store.get_worktree(worktree_id).await? else {
        return Ok(());
    };
    if worktree.workspace_id != workspace.id {
        return Ok(());
    }
    let vcs = vcs_driver_for_worktree(&worktree);
    let worktree_root = Path::new(&worktree.root_path);
    vcs.assert_repo(worktree_root).await?;
    let previous_head = match vcs.kind() {
        VcsKind::Jj => {
            let Some(expected_head) = entry.head_commit_sha.as_deref() else {
                return Ok(());
            };
            let current_head = vcs
                .rev_parse_head(worktree_root)
                .await
                .unwrap_or_else(|_| "unknown".to_string());
            if current_head.trim() != expected_head.trim() {
                return Ok(());
            }
            current_head
        }
        _ => {
            let dirty = vcs.status_porcelain(worktree_root).await?;
            if !dirty.is_empty() {
                return Ok(());
            }
            vcs.rev_parse_head(worktree_root)
                .await
                .unwrap_or_else(|_| "unknown".to_string())
        }
    };
    if vcs.kind() == VcsKind::Git {
        reset_worktree_to_commit(state, entry, &worktree.root_path, commit_sha).await?;
    } else {
        reset_worktree_to_revision(vcs.as_ref(), &worktree.root_path, commit_sha).await?;
    }
    let updated = store
        .update_worktree_base_commit(worktree_id, commit_sha)
        .await?;
    if !updated {
        return Ok(());
    }
    if let Some(session_id) = entry.session_id {
        emit_merge_queue_sync_notice(
            state,
            session_id,
            &worktree,
            &entry.target_branch,
            &previous_head,
            commit_sha,
        )
        .await?;
    }
    Ok(())
}

async fn emit_merge_queue_sync_notice(
    state: &Arc<AppState>,
    session_id: SessionId,
    worktree: &Worktree,
    target_branch: &str,
    previous_commit_sha: &str,
    commit_sha: &str,
) -> Result<()> {
    let previous_short = previous_commit_sha.get(0..8).unwrap_or(previous_commit_sha);
    let short_sha = commit_sha.get(0..8).unwrap_or(commit_sha);
    let message = format!(
        "merge queue applied; reset worktree from {previous_short} to {target_branch} ({short_sha})"
    );
    let store = state.store_for_session(session_id).await?;
    let notice = store
        .append_session_event(
            session_id,
            None,
            None,
            SessionEventType::Notice,
            serde_json::json!({
                "kind": "merge_queue_sync",
                "message": message,
                "worktree_id": worktree.id.0.to_string(),
                "target_branch": target_branch,
                "previous_commit_sha": previous_commit_sha,
                "commit_sha": commit_sha,
                "base_revision": commit_sha,
                "base_commit_sha": commit_sha,
            }),
        )
        .await?;
    state.publish_event(notice).await;
    Ok(())
}

async fn emit_merge_queue_canonical_sync_notice(
    state: &Arc<AppState>,
    session_id: SessionId,
    worktree_id: Option<WorktreeId>,
    target_branch: &str,
    commit_sha: &str,
    status: &str,
    message: &str,
) -> Result<()> {
    let store = state.store_for_session(session_id).await?;
    let notice = store
        .append_session_event(
            session_id,
            None,
            None,
            SessionEventType::Notice,
            serde_json::json!({
                "kind": "merge_queue_canonical_sync",
                "status": status,
                "message": message,
                "worktree_id": worktree_id.map(|id| id.0.to_string()),
                "target_branch": target_branch,
                "commit_sha": commit_sha,
            }),
        )
        .await?;
    state.publish_event(notice).await;
    Ok(())
}

async fn reset_worktree_to_revision(
    vcs: &dyn VcsDriver,
    worktree_path: &str,
    revision: &str,
) -> Result<()> {
    vcs.reset_worktree_to_revision(Path::new(worktree_path), revision)
        .await
        .context("resetting worktree to revision")?;
    Ok(())
}

async fn reset_worktree_to_commit(
    state: &AppState,
    entry: &MergeQueueEntry,
    worktree_path: &str,
    commit_sha: &str,
) -> Result<()> {
    let mut cmd = merge_queue_command(
        state,
        entry,
        "git reset --hard",
        "git",
        Some(Path::new(worktree_path)),
        &[],
    )
    .await;
    let output = cmd
        .arg("-C")
        .arg(worktree_path)
        .args(["reset", "--hard", commit_sha])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git reset --hard")?;
    if !output.status.success() {
        bail!(
            "git reset --hard failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

async fn resolve_target_head(
    vcs: &dyn VcsDriver,
    workspace_root: &str,
    target_branch: &str,
) -> Result<String> {
    match vcs.kind() {
        VcsKind::Git => vcs
            .rev_parse_ref(Path::new(workspace_root), target_branch)
            .await
            .context("resolving target branch"),
        VcsKind::Jj => jj_rev_parse_bookmark(Path::new(workspace_root), target_branch)
            .await
            .context("resolving target bookmark"),
        VcsKind::Hg | VcsKind::Svn | VcsKind::P4 | VcsKind::Other => {
            bail!("merge queue does not support {:?}", vcs.kind());
        }
    }
}

async fn jj_rev_parse_bookmark(root: &Path, bookmark: &str) -> Result<String> {
    let output = Command::new("jj")
        .arg("-R")
        .arg(root)
        .arg("--color=never")
        .arg("--no-pager")
        .args(["log", "-r", bookmark, "--no-graph", "-T", "commit_id"])
        .output()
        .await
        .context("running jj log")?;
    if !output.status.success() {
        bail!(
            "jj log failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let revision = stdout
        .split_whitespace()
        .last()
        .ok_or_else(|| anyhow::anyhow!("jj log produced no revision output"))?;
    Ok(revision.to_string())
}

async fn find_checked_out_worktree_for_branch(
    state: &AppState,
    entry: &MergeQueueEntry,
    workspace_root: &Path,
    target_branch: &str,
) -> Result<Option<String>> {
    let mut cmd = merge_queue_command(
        state,
        entry,
        "git worktree list --porcelain",
        "git",
        Some(workspace_root),
        &[],
    )
    .await;
    let output = cmd
        .arg("-C")
        .arg(workspace_root)
        .args(["worktree", "list", "--porcelain"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git worktree list --porcelain")?;
    if !output.status.success() {
        bail!(
            "git worktree list failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut current_path: Option<String> = None;
    for line in stdout.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            current_path = Some(path.trim().to_string());
            continue;
        }
        if let Some(branch) = line.strip_prefix("branch ") {
            let branch = branch.trim();
            if branch == format!("refs/heads/{target_branch}") {
                if let Some(path) = current_path.clone() {
                    return Ok(Some(path));
                }
            }
            continue;
        }
        if line.trim().is_empty() {
            current_path = None;
        }
    }
    Ok(None)
}

async fn command_for_shell(
    state: &AppState,
    entry: &MergeQueueEntry,
    command: &str,
    workdir: &Path,
    envs: &[(String, String)],
) -> Command {
    if cfg!(windows) {
        let mut cmd = merge_queue_command(state, entry, command, "cmd", Some(workdir), envs).await;
        cmd.args(["/C", command]);
        cmd
    } else {
        let mut cmd = merge_queue_command(state, entry, command, "bash", Some(workdir), envs).await;
        cmd.args(["-lc", command]);
        cmd
    }
}

#[allow(clippy::too_many_arguments)]
async fn update_target_branch(
    state: &AppState,
    entry: &MergeQueueEntry,
    repo_root: &Path,
    workspace_root: &Path,
    vcs_kind: VcsKind,
    target_branch: &str,
    commit_sha: &str,
    expected_old: &str,
) -> std::result::Result<(), QueueError> {
    match vcs_kind {
        VcsKind::Git => {
            let mut cmd =
                merge_queue_command(state, entry, "git update-ref", "git", Some(repo_root), &[])
                    .await;
            let output = cmd
                .arg("-C")
                .arg(repo_root)
                .args([
                    "update-ref",
                    &format!("refs/heads/{target_branch}"),
                    commit_sha,
                    expected_old,
                ])
                .output()
                .await
                .map_err(|e| QueueError::fail(e.to_string(), None, Some(commit_sha.to_string())))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                return Err(QueueError::fail(
                    format!("failed to update target branch: {stderr}"),
                    Some(output.status.code().unwrap_or(1) as i64),
                    Some(commit_sha.to_string()),
                ));
            }
            Ok(())
        }
        VcsKind::Jj => {
            let current = jj_rev_parse_bookmark(workspace_root, target_branch)
                .await
                .map_err(|e| QueueError::fail(e.to_string(), None, Some(commit_sha.to_string())))?;
            if current.trim() != expected_old.trim() {
                return Err(QueueError::fail(
                    format!("target branch advanced (expected {expected_old}, found {current})"),
                    None,
                    Some(commit_sha.to_string()),
                ));
            }
            let mut cmd = merge_queue_command(
                state,
                entry,
                "jj bookmark set",
                "jj",
                Some(workspace_root),
                &[],
            )
            .await;
            let output = cmd
                .arg("-R")
                .arg(workspace_root)
                .arg("--color=never")
                .arg("--no-pager")
                .args(["bookmark", "set", target_branch, "-r", commit_sha])
                .output()
                .await
                .map_err(|e| QueueError::fail(e.to_string(), None, Some(commit_sha.to_string())))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                return Err(QueueError::fail(
                    format!("failed to update target bookmark: {stderr}"),
                    Some(output.status.code().unwrap_or(1) as i64),
                    Some(commit_sha.to_string()),
                ));
            }
            Ok(())
        }
        VcsKind::Hg | VcsKind::Svn | VcsKind::P4 | VcsKind::Other => Err(QueueError::fail(
            format!("merge queue does not support {vcs_kind:?} branches"),
            None,
            Some(commit_sha.to_string()),
        )),
    }
}

#[allow(clippy::too_many_arguments)]
async fn push_target_branch(
    state: &AppState,
    entry: &MergeQueueEntry,
    repo_root: &Path,
    workspace_root: &Path,
    vcs_kind: VcsKind,
    remote: &str,
    target_branch: &str,
    push_branch: &str,
    commit_sha: &str,
) -> Result<()> {
    match vcs_kind {
        VcsKind::Git => {
            let mut cmd =
                merge_queue_command(state, entry, "git push", "git", Some(repo_root), &[]).await;
            let output = cmd
                .arg("-C")
                .arg(repo_root)
                .args(["push", remote, &format!("{target_branch}:{push_branch}")])
                .output()
                .await
                .context("running git push")?;
            if !output.status.success() {
                bail!(
                    "git push failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            Ok(())
        }
        VcsKind::Jj => {
            if target_branch != push_branch {
                let mut cmd = merge_queue_command(
                    state,
                    entry,
                    "jj bookmark set",
                    "jj",
                    Some(workspace_root),
                    &[],
                )
                .await;
                let output = cmd
                    .arg("-R")
                    .arg(workspace_root)
                    .arg("--color=never")
                    .arg("--no-pager")
                    .args(["bookmark", "set", push_branch, "-r", commit_sha])
                    .output()
                    .await
                    .context("running jj bookmark set")?;
                if !output.status.success() {
                    bail!(
                        "jj bookmark set failed: {}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
            }
            let mut cmd =
                merge_queue_command(state, entry, "jj git push", "jj", Some(workspace_root), &[])
                    .await;
            let output = cmd
                .arg("-R")
                .arg(workspace_root)
                .arg("--color=never")
                .arg("--no-pager")
                .args(["git", "push", "--remote", remote, "--bookmark", push_branch])
                .output()
                .await
                .context("running jj git push")?;
            if !output.status.success() {
                bail!(
                    "jj git push failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            Ok(())
        }
        VcsKind::Hg | VcsKind::Svn | VcsKind::P4 | VcsKind::Other => {
            bail!("merge queue does not support {vcs_kind:?} pushes");
        }
    }
}

#[derive(Debug)]
enum QueueError {
    Conflict {
        message: String,
    },
    Failed {
        message: String,
        exit_code: Option<i64>,
        result_commit_sha: Option<String>,
    },
}

impl QueueError {
    fn fail(message: String, exit_code: Option<i64>, result_commit_sha: Option<String>) -> Self {
        QueueError::Failed {
            message,
            exit_code,
            result_commit_sha,
        }
    }
}

fn emit_merge_queue_tool_event(
    state: &AppState,
    entry: &MergeQueueEntry,
    command: &str,
    workdir: Option<&Path>,
    used_tool_slice: bool,
) {
    let mut event = OpsEvent::new("info", "merge_queue_tool_exec");
    event.session_id = entry.session_id.map(|id| id.0.to_string());
    event.worktree_id = entry.worktree_id.map(|id| id.0.to_string());
    event.tool_kind = Some("merge_queue".to_string());
    let workdir_str = workdir.map(|dir| dir.to_string_lossy().to_string());
    event.cwd = workdir_str.clone();
    event.worktree_root = workdir_str;
    event.meta = Some(serde_json::json!({
        "entry_id": entry.id.0.to_string(),
        "command": command,
        "tool_slice": used_tool_slice,
        "slice": TOOL_SLICE_UNIT,
    }));
    state.telemetry.ops_events.emit(event);
}

async fn merge_queue_proxy_env(state: &AppState, entry: &MergeQueueEntry) -> Vec<(String, String)> {
    let settings = settings::load_settings(&state.core.data_root).await;
    let network_profiles = settings.network_profiles.unwrap_or_default();
    let network_profile = network_profiles.profile(NetworkContext::MergeQueue);
    let env = state
        .execution
        .egress_proxy
        .proxy_env_for_context(
            entry.workspace_id,
            NetworkContext::MergeQueue,
            network_profile,
            "127.0.0.1",
        )
        .await;
    env.into_iter().collect()
}

async fn merge_queue_command(
    state: &AppState,
    entry: &MergeQueueEntry,
    command_label: &str,
    program: &str,
    workdir: Option<&Path>,
    envs: &[(String, String)],
) -> Command {
    let proxy_env = merge_queue_proxy_env(state, entry).await;
    let mut merged: HashMap<String, String> = envs.iter().cloned().collect();
    for (key, value) in proxy_env {
        merged.insert(key, value);
    }
    let merged: Vec<(String, String)> = merged.into_iter().collect();
    let (cmd, used_tool_slice) = tool_slice_command(program, workdir, &merged).await;
    emit_merge_queue_tool_event(state, entry, command_label, workdir, used_tool_slice);
    cmd
}

#[cfg(target_os = "linux")]
async fn tool_slice_command(
    program: &str,
    workdir: Option<&Path>,
    envs: &[(String, String)],
) -> (Command, bool) {
    if systemd_run_available().await {
        let mut cmd = Command::new("systemd-run");
        cmd.arg("--user")
            .arg("--slice")
            .arg(TOOL_SLICE_UNIT)
            .arg("--quiet")
            .arg("--pipe")
            .arg("--wait");
        if let Some(dir) = workdir {
            cmd.arg("--working-directory").arg(dir);
        }
        for (key, value) in envs {
            cmd.arg("--setenv").arg(format!("{key}={value}"));
        }
        cmd.arg("--").arg(program);
        return (cmd, true);
    }

    let mut cmd = Command::new(program);
    if let Some(dir) = workdir {
        cmd.current_dir(dir);
    }
    for (key, value) in envs {
        cmd.env(key, value);
    }
    (cmd, false)
}

#[cfg(not(target_os = "linux"))]
async fn tool_slice_command(
    program: &str,
    workdir: Option<&Path>,
    envs: &[(String, String)],
) -> (Command, bool) {
    let mut cmd = Command::new(program);
    if let Some(dir) = workdir {
        cmd.current_dir(dir);
    }
    for (key, value) in envs {
        cmd.env(key, value);
    }
    (cmd, false)
}

#[cfg(target_os = "linux")]
async fn systemd_run_available() -> bool {
    static SYSTEMD_RUN_AVAILABLE: OnceLock<bool> = OnceLock::new();
    if let Some(value) = SYSTEMD_RUN_AVAILABLE.get() {
        return *value;
    }

    let available = {
        let output = match Command::new("systemd-run").arg("--version").output().await {
            Ok(output) => output,
            Err(_) => return false,
        };
        if !output.status.success() {
            false
        } else {
            let output = Command::new("systemctl")
                .arg("--user")
                .arg("show-environment")
                .output()
                .await;
            output.map(|o| o.status.success()).unwrap_or(false)
        }
    };

    let _ = SYSTEMD_RUN_AVAILABLE.set(available);
    if !available {
        tracing::warn!(
            "systemd-run unavailable; merge queue commands will run without tool slice isolation"
        );
    }
    available
}
