use std::collections::HashMap;
use std::path::Path;
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
    MergeQueueRunStatus, VcsKind, Workspace, Worktree,
};
use ctx_fs::git::git_status_porcelain;
use ctx_fs::vcs::{self, VcsDriver};

use crate::daemon::AppState;
use crate::ops_events::OpsEvent;
#[cfg(target_os = "linux")]
use crate::tool_cgroup::TOOL_SLICE_UNIT;
#[cfg(not(target_os = "linux"))]
const TOOL_SLICE_UNIT: &str = "ctx-tools.slice";
use crate::workspace_config::{load_merge_queue_config, MergeQueueCanonicalSync, MergeQueueConfig};

mod context;
mod execution;
mod storage;
mod sync;
mod target;

use context::*;
use execution::*;

use storage::{merge_queue_log_path, open_log_file, write_log_line, write_patch_file};
use sync::maybe_update_worktree_base_commit_for_path;

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

pub async fn get_workspace_merge_queue_entry(
    state: &AppState,
    workspace_id: WorkspaceId,
    entry_id: MergeQueueEntryId,
) -> Result<MergeQueueEntry> {
    let store = state.store_for_workspace(workspace_id).await?;
    store
        .get_merge_queue_entry(entry_id)
        .await?
        .filter(|entry| entry.workspace_id == workspace_id)
        .ok_or_else(|| anyhow::anyhow!("merge queue entry not found"))
}

async fn list_queued_entries_for_workspace(
    state: &AppState,
    workspace_id: WorkspaceId,
) -> Result<Vec<MergeQueueEntry>> {
    let store = state.core.stores.workspace(workspace_id).await?;
    let mut entries = store.list_queued_merge_queue_entries().await?;
    entries.sort_by_key(|entry| entry.created_at);
    Ok(entries)
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
    let workspace_store = state.store_for_workspace(workspace.id).await?;
    let config = load_merge_queue_config(&workspace_store).await?;
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
            bootstrap_command: None,
            bootstrap_script_path: None,
        };
        workspace_store
            .insert_worktree(worktree_record.clone())
            .await?;
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
    workspace_store.create_merge_queue_entry(&entry).await?;
    let _ = state.transport.merge_queue_schedule_tx.send(workspace.id);
    state.transport.merge_queue_notify.notify_one();
    let entry = wait_for_merge_queue_completion(state, entry.workspace_id, entry.id).await?;
    ensure_merge_queue_success(&entry)?;
    Ok(entry)
}

pub async fn cancel_merge_queue_entry(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    entry_id: MergeQueueEntryId,
) -> Result<MergeQueueEntry> {
    let mut entry = get_workspace_merge_queue_entry(state, workspace_id, entry_id).await?;
    let store = state.store_for_workspace(workspace_id).await?;
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
    workspace_id: WorkspaceId,
    entry_id: MergeQueueEntryId,
) -> Result<MergeQueueEntry> {
    let mut entry = get_workspace_merge_queue_entry(state, workspace_id, entry_id).await?;
    let store = state.store_for_workspace(workspace_id).await?;
    match entry.status {
        MergeQueueEntryStatus::Failed | MergeQueueEntryStatus::Conflict => {
            entry.status = MergeQueueEntryStatus::Queued;
            entry.error_message = None;
            entry.result_commit_sha = None;
            entry.updated_at = Utc::now();
            store.update_merge_queue_entry(&entry).await?;
            let _ = state.transport.merge_queue_schedule_tx.send(workspace_id);
            state.transport.merge_queue_notify.notify_one();
            state.transport.merge_queue_notify.notify_waiters();
            Ok(entry)
        }
        _ => Ok(entry),
    }
}

pub fn spawn_merge_queue_runner(state: Arc<AppState>) {
    tokio::spawn(async move {
        let Some(mut rx) = state.transport.merge_queue_schedule_rx.lock().await.take() else {
            tracing::warn!("merge queue runner already started");
            return;
        };
        while let Some(workspace_id) = rx.recv().await {
            schedule_workspace_drain(&state, workspace_id).await;
        }
    });
}

pub(crate) async fn schedule_workspace_if_enabled_and_queued(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
) -> Result<bool> {
    let store = state.core.stores.workspace(workspace_id).await?;
    let cfg = load_merge_queue_config(&store).await?;
    if !cfg.enabled || store.list_queued_merge_queue_entries().await?.is_empty() {
        return Ok(false);
    }
    let _ = state.transport.merge_queue_schedule_tx.send(workspace_id);
    Ok(true)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkspaceDrainStep {
    Continue,
    Idle,
    Disabled,
    MissingWorkspace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkspaceDrainStop {
    Idle,
    Disabled,
    MissingWorkspace,
    Error,
}

async fn begin_workspace_drain(state: &AppState, workspace_id: WorkspaceId) -> bool {
    let mut schedule_state = state.transport.merge_queue_state.lock().await;
    let inserted = schedule_state.running.insert(workspace_id);
    if inserted {
        schedule_state.pending.remove(&workspace_id);
    } else {
        schedule_state.pending.insert(workspace_id);
    }
    inserted
}

async fn finish_workspace_drain(state: &AppState, workspace_id: WorkspaceId) -> bool {
    let mut schedule_state = state.transport.merge_queue_state.lock().await;
    schedule_state.running.remove(&workspace_id);
    schedule_state.pending.remove(&workspace_id)
}

async fn schedule_workspace_drain(state: &Arc<AppState>, workspace_id: WorkspaceId) {
    let should_spawn = begin_workspace_drain(state.as_ref(), workspace_id).await;
    if !should_spawn {
        return;
    }
    let state = Arc::clone(state);
    tokio::spawn(async move {
        let stop = loop {
            let step = match run_next_entry_for_workspace(&state, workspace_id).await {
                Ok(step) => step,
                Err(err) => {
                    tracing::warn!(workspace_id = %workspace_id.0, "merge queue runner error: {err:#}");
                    break WorkspaceDrainStop::Error;
                }
            };
            match step {
                WorkspaceDrainStep::Continue => continue,
                WorkspaceDrainStep::Idle => break WorkspaceDrainStop::Idle,
                WorkspaceDrainStep::Disabled => break WorkspaceDrainStop::Disabled,
                WorkspaceDrainStep::MissingWorkspace => break WorkspaceDrainStop::MissingWorkspace,
            }
        };

        if finish_workspace_drain(state.as_ref(), workspace_id).await {
            let _ = state.transport.merge_queue_schedule_tx.send(workspace_id);
            return;
        }

        let _ = reschedule_workspace_after_drain(&state, workspace_id, stop).await;
    });
}

async fn reschedule_workspace_after_drain(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    stop: WorkspaceDrainStop,
) -> bool {
    if stop == WorkspaceDrainStop::MissingWorkspace {
        return false;
    }
    if stop == WorkspaceDrainStop::Disabled {
        tracing::debug!(
            workspace_id = %workspace_id.0,
            "merge queue drain dormant because workspace queue is disabled"
        );
        return false;
    }
    match schedule_workspace_if_enabled_and_queued(state, workspace_id).await {
        Ok(true) => true,
        Ok(false) => false,
        Err(err) => {
            tracing::warn!(
                workspace_id = %workspace_id.0,
                "failed to re-check queued merge queue entries after drain: {err:#}"
            );
            false
        }
    }
}

async fn run_next_entry_for_workspace(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
) -> Result<WorkspaceDrainStep> {
    let workspace = match state.global_store().get_workspace(workspace_id).await? {
        Some(workspace) => workspace,
        None => return Ok(WorkspaceDrainStep::MissingWorkspace),
    };
    let store = state.core.stores.workspace(workspace.id).await?;
    let cfg = load_merge_queue_config(&store).await?;
    if !cfg.enabled {
        cancel_queued_entries_for_disabled_workspace(state, &store, workspace.id).await?;
        return Ok(WorkspaceDrainStep::Disabled);
    }

    let Some(mut entry) = list_queued_entries_for_workspace(state, workspace_id)
        .await?
        .into_iter()
        .next()
    else {
        return Ok(WorkspaceDrainStep::Idle);
    };

    let now = Utc::now();
    let claimed = store.claim_merge_queue_entry(entry.id, now).await?;
    if !claimed {
        return Ok(WorkspaceDrainStep::Continue);
    }
    entry.status = MergeQueueEntryStatus::Running;
    entry.updated_at = now;
    run_entry(state, &workspace, entry, &cfg).await?;
    Ok(WorkspaceDrainStep::Continue)
}

async fn cancel_queued_entries_for_disabled_workspace(
    state: &Arc<AppState>,
    store: &ctx_store::Store,
    workspace_id: WorkspaceId,
) -> Result<()> {
    let queued = store.list_queued_merge_queue_entries().await?;
    if queued.is_empty() {
        return Ok(());
    }

    let now = Utc::now();
    for mut entry in queued {
        entry.status = MergeQueueEntryStatus::Cancelled;
        entry.error_message = Some("merge queue disabled while entry was queued".to_string());
        entry.updated_at = now;
        store.update_merge_queue_entry(&entry).await?;
    }

    tracing::debug!(
        workspace_id = %workspace_id.0,
        cancelled = true,
        "cancelled queued merge queue entries because the workspace queue is disabled"
    );
    state.transport.merge_queue_notify.notify_waiters();
    Ok(())
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
    workspace_id: WorkspaceId,
    entry_id: MergeQueueEntryId,
) -> Result<MergeQueueEntry> {
    let store = state.store_for_workspace(workspace_id).await?;
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

async fn merge_queue_command(
    state: &AppState,
    entry: &MergeQueueEntry,
    command_label: &str,
    program: &str,
    workdir: Option<&Path>,
    envs: &[(String, String)],
) -> Command {
    let merged: HashMap<String, String> = envs.iter().cloned().collect();
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

#[cfg(test)]
mod tests;
