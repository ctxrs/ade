use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use chrono::Utc;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use ctx_core::ids::{MergeQueueEntryId, MergeQueueRunId, SessionId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    MergeQueueEntry, MergeQueueEntryStatus, MergeQueuePatchSource, MergeQueueRun,
    MergeQueueRunStatus, SessionEventType, Workspace, Worktree,
};
use ctx_fs::git::{
    assert_git_repo, git_is_ancestor, git_merge_base, git_status_porcelain, rev_parse_ref,
};
use ctx_fs::patch::build_worktree_patch;
use ctx_fs::worktrees::{create_worktree, remove_worktree};

use crate::daemon::AppState;
use crate::workspace_config::{load_merge_queue_config, MergeQueueConfig};

#[derive(Debug, Clone)]
pub struct MergeQueueSubmitParams {
    pub session_id: Option<SessionId>,
    pub worktree_id: Option<WorktreeId>,
    pub target_branch: Option<String>,
    pub message: Option<String>,
}

pub async fn submit_merge_queue_entry(
    state: &Arc<AppState>,
    params: MergeQueueSubmitParams,
) -> Result<MergeQueueEntry> {
    let (workspace, worktree) =
        resolve_workspace_context(state, params.session_id, params.worktree_id).await?;
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

    let worktree = worktree
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("worktree is required to submit to the merge queue"))?;
    assert_git_repo(&worktree.root_path).await?;
    let dirty = git_status_porcelain(&worktree.root_path).await?;
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
    let up_to_date = git_is_ancestor(&worktree.root_path, &target_branch, "HEAD").await?;
    if !up_to_date {
        bail!("worktree HEAD is behind target branch {target_branch}; sync and retry");
    }
    let merge_base = git_merge_base(&worktree.root_path, &target_branch, "HEAD").await?;
    let worktree_patch = build_worktree_patch(&worktree.root_path, &merge_base).await?;
    if worktree_patch.patch.trim().is_empty() {
        bail!("no changes detected; nothing to submit");
    }
    let patch_source = MergeQueuePatchSource::Generated;
    let base_commit_sha = Some(worktree_patch.base_commit_sha);
    let head_commit_sha = Some(worktree_patch.head_commit_sha);
    let patch_text = worktree_patch.patch;

    let entry_id = MergeQueueEntryId::new();
    let patch_path = write_patch_file(&state.data_root, entry_id, &patch_text).await?;
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
    state.store.create_merge_queue_entry(&entry).await?;
    state.merge_queue_notify.notify_one();
    let entry = wait_for_merge_queue_completion(state, entry.id).await?;
    ensure_merge_queue_success(&entry)?;
    Ok(entry)
}

pub async fn cancel_merge_queue_entry(
    state: &Arc<AppState>,
    entry_id: MergeQueueEntryId,
) -> Result<MergeQueueEntry> {
    let mut entry = state
        .store
        .get_merge_queue_entry(entry_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("merge queue entry not found"))?;
    match entry.status {
        MergeQueueEntryStatus::Queued => {
            entry.status = MergeQueueEntryStatus::Cancelled;
            entry.updated_at = Utc::now();
            state.store.update_merge_queue_entry(&entry).await?;
            state.merge_queue_notify.notify_waiters();
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
    let mut entry = state
        .store
        .get_merge_queue_entry(entry_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("merge queue entry not found"))?;
    match entry.status {
        MergeQueueEntryStatus::Failed | MergeQueueEntryStatus::Conflict => {
            entry.status = MergeQueueEntryStatus::Queued;
            entry.error_message = None;
            entry.result_commit_sha = None;
            entry.updated_at = Utc::now();
            state.store.update_merge_queue_entry(&entry).await?;
            state.merge_queue_notify.notify_one();
            state.merge_queue_notify.notify_waiters();
            Ok(entry)
        }
        _ => Ok(entry),
    }
}

pub fn spawn_merge_queue_runner(state: Arc<AppState>) {
    let notify = state.merge_queue_notify.clone();
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
    let entries = state.store.list_queued_merge_queue_entries().await?;
    for mut entry in entries {
        let workspace = match state.store.get_workspace(entry.workspace_id).await? {
            Some(ws) => ws,
            None => continue,
        };
        let cfg = load_merge_queue_config(Path::new(&workspace.root_path)).await?;
        if !cfg.enabled {
            continue;
        }
        let now = Utc::now();
        let claimed = state.store.claim_merge_queue_entry(entry.id, now).await?;
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
    let run_id = MergeQueueRunId::new();
    let log_path = merge_queue_log_path(&state.data_root, run_id);
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
    state.store.create_merge_queue_run(&run).await?;

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
            state.store.update_merge_queue_entry(&entry).await?;
            state.store.update_merge_queue_run(&run).await?;
            state.merge_queue_notify.notify_waiters();
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
            state.store.update_merge_queue_entry(&entry).await?;
            state.store.update_merge_queue_run(&run).await?;
            state.merge_queue_notify.notify_waiters();
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
            state.store.update_merge_queue_entry(&entry).await?;
            state.store.update_merge_queue_run(&run).await?;
            state.merge_queue_notify.notify_waiters();
        }
    }

    Ok(())
}

async fn wait_for_merge_queue_completion(
    state: &Arc<AppState>,
    entry_id: MergeQueueEntryId,
) -> Result<MergeQueueEntry> {
    let notify = state.merge_queue_notify.clone();
    loop {
        let entry = state
            .store
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
    assert_git_repo(&workspace.root_path)
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;

    let target_head = rev_parse_ref(&workspace.root_path, &entry.target_branch)
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
    let worktree_path = merge_queue_worktree_path(&state.data_root, workspace.id, entry.id);
    let worktree_branch = format!("ctx-merge-queue/{}", entry.id.0);
    let _ = remove_worktree(&workspace.root_path, &worktree_path).await;
    let _ = delete_branch(&workspace.root_path, &worktree_branch).await;
    create_worktree(
        &workspace.root_path,
        &worktree_path,
        &target_head,
        &worktree_branch,
    )
    .await
    .map_err(|e| QueueError::fail(e.to_string(), None, None))?;

    let result = async {
        let patch = read_patch_file(&entry.patch_path)
            .await
            .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
        write_log_line(log_file, "apply patch\n")
            .await
            .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
        apply_patch(&worktree_path, &patch).await?;

        if !has_staged_changes(&worktree_path).await? {
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
        commit_changes(&worktree_path, message, log_file).await?;
        let commit_sha = rev_parse_ref(&worktree_path, "HEAD")
            .await
            .map_err(|e| QueueError::fail(e.to_string(), None, None))?;

        for cmd in &cfg.verify_commands {
            run_verify_command(&worktree_path, entry, cmd, log_file).await?;
        }

        write_log_line(
            log_file,
            &format!("advance target branch {}\n", entry.target_branch),
        )
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
        update_target_branch(
            &workspace.root_path,
            &entry.target_branch,
            &commit_sha,
            &target_head,
        )
        .await?;

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
                &workspace.root_path,
                &cfg.push_remote,
                &entry.target_branch,
                &cfg.push_branch,
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

        Ok(commit_sha)
    }
    .await;

    let _ = remove_worktree(&workspace.root_path, &worktree_path).await;
    let _ = delete_branch(&workspace.root_path, &worktree_branch).await;
    result
}

async fn resolve_workspace_context(
    state: &Arc<AppState>,
    session_id: Option<SessionId>,
    worktree_id: Option<WorktreeId>,
) -> Result<(Workspace, Option<Worktree>)> {
    if let Some(worktree_id) = worktree_id {
        let worktree = state
            .store
            .get_worktree(worktree_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("worktree not found"))?;
        let workspace = state
            .store
            .get_workspace(worktree.workspace_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("workspace not found"))?;
        return Ok((workspace, Some(worktree)));
    }

    let session_id = session_id.ok_or_else(|| anyhow::anyhow!("session_id is required"))?;
    let session = state
        .store
        .get_session(session_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("session not found"))?;
    let workspace = state
        .store
        .get_workspace(session.workspace_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("workspace not found"))?;
    let worktree = state.store.get_worktree(session.worktree_id).await?;
    Ok((workspace, worktree))
}

fn merge_queue_root(data_root: &Path) -> PathBuf {
    data_root.join("merge-queue")
}

fn merge_queue_patch_path(data_root: &Path, entry_id: MergeQueueEntryId) -> PathBuf {
    merge_queue_root(data_root)
        .join("patches")
        .join(format!("{}.patch", entry_id.0))
}

fn merge_queue_log_path(data_root: &Path, run_id: MergeQueueRunId) -> PathBuf {
    merge_queue_root(data_root)
        .join("logs")
        .join(format!("{}.log", run_id.0))
}

fn merge_queue_worktree_path(
    data_root: &Path,
    workspace_id: WorkspaceId,
    entry_id: MergeQueueEntryId,
) -> PathBuf {
    merge_queue_root(data_root)
        .join("worktrees")
        .join(workspace_id.0.to_string())
        .join(entry_id.0.to_string())
}

async fn write_patch_file(
    data_root: &Path,
    entry_id: MergeQueueEntryId,
    patch: &str,
) -> Result<PathBuf> {
    let path = merge_queue_patch_path(data_root, entry_id);
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

async fn apply_patch(worktree_path: &Path, patch: &str) -> std::result::Result<(), QueueError> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(worktree_path)
        .arg("apply")
        .arg("--index")
        .arg("--3way")
        .arg("--whitespace=nowarn")
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
    Ok(())
}

async fn has_staged_changes(worktree_path: &Path) -> std::result::Result<bool, QueueError> {
    let status = Command::new("git")
        .arg("-C")
        .arg(worktree_path)
        .args(["diff", "--cached", "--quiet"])
        .status()
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
    Ok(!status.success())
}

async fn commit_changes(
    worktree_path: &Path,
    message: &str,
    log_file: &mut fs::File,
) -> std::result::Result<(), QueueError> {
    let output = Command::new("git")
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

async fn run_verify_command(
    worktree_path: &Path,
    entry: &MergeQueueEntry,
    command: &str,
    log_file: &mut fs::File,
) -> std::result::Result<(), QueueError> {
    write_log_line(log_file, &format!("verify: {command}\n"))
        .await
        .map_err(|e| QueueError::fail(e.to_string(), None, None))?;
    let mut cmd = command_for_shell(command);
    cmd.current_dir(worktree_path)
        .stdin(Stdio::null())
        .env("CTX_MERGE_QUEUE_ENTRY_ID", entry.id.0.to_string())
        .env("CTX_WORKTREE_ROOT", worktree_path)
        .env("CTX_TARGET_BRANCH", &entry.target_branch);
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
    let Some(worktree) = state.store.get_worktree(worktree_id).await? else {
        return Ok(());
    };
    if worktree.workspace_id != workspace.id {
        return Ok(());
    }
    assert_git_repo(&worktree.root_path).await?;
    let dirty = git_status_porcelain(&worktree.root_path).await?;
    if !dirty.is_empty() {
        return Ok(());
    }
    let previous_head = rev_parse_ref(&worktree.root_path, "HEAD")
        .await
        .unwrap_or_else(|_| "unknown".to_string());
    reset_worktree_to_commit(&worktree.root_path, commit_sha).await?;
    let updated = state
        .store
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
    let notice = state
        .store
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
                "base_commit_sha": commit_sha,
            }),
        )
        .await?;
    state.publish_event(notice).await;
    Ok(())
}

async fn reset_worktree_to_commit(worktree_path: &str, commit_sha: &str) -> Result<()> {
    let output = Command::new("git")
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

fn command_for_shell(command: &str) -> Command {
    if cfg!(windows) {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", command]);
        cmd
    } else {
        let mut cmd = Command::new("bash");
        cmd.args(["-lc", command]);
        cmd
    }
}

async fn update_target_branch(
    workspace_root: &str,
    target_branch: &str,
    commit_sha: &str,
    expected_old: &str,
) -> std::result::Result<(), QueueError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_root)
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

async fn push_target_branch(
    workspace_root: &str,
    remote: &str,
    target_branch: &str,
    push_branch: &str,
) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_root)
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

async fn delete_branch(workspace_root: &str, branch: &str) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_root)
        .args(["branch", "-D", branch])
        .output()
        .await
        .context("running git branch -D")?;
    if !output.status.success() {
        bail!(
            "git branch -D failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
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
