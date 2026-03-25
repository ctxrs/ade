use std::collections::HashSet;
use std::path::{Path as StdPath, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

#[path = "tasks/creation.rs"]
mod creation;
mod handlers;
pub(in crate::api) use creation::*;
pub(in crate::api) use handlers::*;

use super::errors::ApiErrorResp;
use super::sessions::schedule_session_title_generation;
use super::shared::session_root_kind_for_worktree;
use crate::attachments;
use crate::daemon::AppState;
use crate::execution_effective;
use crate::logs;
use crate::ops_events::OpsEvent;
use crate::scheduler::SchedulerCommand;
use crate::settings::{ExecutionMode, ExecutionSettings};
use crate::telemetry::TelemetryEvent;
use crate::vcs_hooks;
use crate::worktree_bootstrap;
use ctx_core::ids::{MessageId, RunId, SessionId, TaskId, TurnId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    ExecutionEnvironment, Message, MessageDelivery, MessageRole, SandboxBinding, SandboxProfile,
    SandboxRuntimeFamily, Session, SessionEventType, SessionTurn, SessionTurnStatus, Task,
    TaskDeltaKind, VcsKind, Workspace, WorkspaceArchivedPage, WorkspaceIndexCursor, Worktree,
};
use ctx_fs::git::delete_branch;
use ctx_fs::vcs;
use ctx_fs::worktrees::{create_worktree, managed_worktree_path};
use ctx_store::is_unique_constraint_violation;

const GLOBAL_INDEX_WRITE_RETRY_LIMIT: usize = 3;
const GLOBAL_INDEX_WRITE_RETRY_BASE_MS: u64 = 40;

fn execution_environment_from_settings(settings: &ExecutionSettings) -> ExecutionEnvironment {
    match settings.mode {
        ExecutionMode::Host => ExecutionEnvironment::Host,
        ExecutionMode::Sandbox => ExecutionEnvironment::Sandbox,
    }
}

pub(in crate::api) async fn provision_worktree_for_execution(
    state: &Arc<AppState>,
    workspace: &Workspace,
    worktree_id: WorktreeId,
    base_commit_sha: &str,
    branch_name: &str,
    effective: &ExecutionSettings,
) -> anyhow::Result<(PathBuf, Option<SandboxBinding>)> {
    let canonical_root = managed_worktree_path(&state.core.data_root, workspace.id, worktree_id);
    if let Some(parent) = canonical_root.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    create_worktree(
        &workspace.root_path,
        &canonical_root,
        base_commit_sha,
        branch_name,
    )
    .await?;

    if !matches!(effective.mode, ExecutionMode::Sandbox)
        || !matches!(
            effective.container.mount_mode,
            crate::settings::ContainerMountMode::DiskIsolated
        )
    {
        return Ok((canonical_root, None));
    }

    state
        .execution
        .harness
        .ensure_workspace_container(workspace, effective, &state.core.daemon_url)
        .await?;

    let (live_worktree_root, runtime_family, host_projection_root) = if matches!(
        effective.container.runtime,
        crate::settings::ContainerRuntimeKind::SharedVmContainer
    ) {
        let guest_worktree =
            crate::workspace_runtime::ensure_avf_linux_guest_worktree_from_host_copy(
                &state.core.data_root,
                workspace.id,
                worktree_id,
                &canonical_root,
                base_commit_sha,
                branch_name,
                None,
            )
            .await?;
        let live_root = crate::disk_isolated::ensure_worktree_from_host_copy(
            &state.core.data_root,
            workspace.id,
            worktree_id,
            &guest_worktree.host_shadow_root,
            base_commit_sha,
            branch_name,
        )
        .await?;
        (
            live_root,
            SandboxRuntimeFamily::SharedVmContainer,
            Some(
                guest_worktree
                    .host_shadow_root
                    .to_string_lossy()
                    .to_string(),
            ),
        )
    } else {
        let live_root = crate::disk_isolated::ensure_worktree_from_host_copy(
            &state.core.data_root,
            workspace.id,
            worktree_id,
            &canonical_root,
            base_commit_sha,
            branch_name,
        )
        .await?;
        (live_root, SandboxRuntimeFamily::NativeContainer, None)
    };

    Ok((
        canonical_root,
        Some(SandboxBinding {
            worktree_id,
            workspace_id: workspace.id,
            runtime_family,
            profile: SandboxProfile::Standard,
            live_workspace_root: crate::harness_runtime::CTX_CONTAINER_WORKSPACE_ROOT.to_string(),
            live_worktree_root: live_worktree_root.to_string_lossy().to_string(),
            execution_settings_json: Some(serde_json::to_string(effective)?),
            container_name: Some(crate::harness_runtime::workspace_container_name(
                workspace.id,
            )),
            host_projection_root,
            created_at: Utc::now(),
        }),
    ))
}

fn is_transient_store_error(err: &anyhow::Error) -> bool {
    let msg = err.to_string().to_lowercase();
    msg.contains("database is locked")
        || msg.contains("sqlite_busy")
        || msg.contains("database is busy")
}

async fn retry_global_index_write<Fut>(mut op: impl FnMut() -> Fut) -> Result<(), anyhow::Error>
where
    Fut: std::future::Future<Output = Result<(), anyhow::Error>>,
{
    let mut attempt = 0usize;
    loop {
        match op().await {
            Ok(()) => return Ok(()),
            Err(err) => {
                if !is_transient_store_error(&err) || attempt >= GLOBAL_INDEX_WRITE_RETRY_LIMIT {
                    return Err(err);
                }
                attempt += 1;
                let backoff_ms = GLOBAL_INDEX_WRITE_RETRY_BASE_MS.saturating_mul(attempt as u64);
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            }
        }
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct UpdateTaskTitleReq {
    title: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct CreateTaskReq {
    #[serde(default)]
    id: Option<String>,
    title: String,
    description: Option<String>,
    #[serde(default = "default_true")]
    create_default_session: bool,
}

fn default_true() -> bool {
    true
}

fn task_request_matches(existing: &Task, title: &str, description: &Option<String>) -> bool {
    existing.title == title && existing.description.as_deref() == description.as_deref()
}

pub(super) async fn update_task_title(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateTaskTitleReq>,
) -> Result<Json<Task>, (StatusCode, Json<ApiErrorResp>)> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid task id".to_string(),
            }),
        )
    })?);
    let title = req.title.trim().to_string();
    if title.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "title is required".to_string(),
            }),
        ));
    }
    if title.len() > 120 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "title is too long".to_string(),
            }),
        ));
    }

    let store = state.store_for_task(task_id).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let updated = store.update_task_title(task_id, title).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    if !updated {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "task not found".to_string(),
            }),
        ));
    }

    let task = match store.get_task_with_activity(task_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })? {
        Some(task) => task,
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "task not found".to_string(),
                }),
            ))
        }
    };

    let _ = state
        .emit_workspace_task_delta(task.clone(), TaskDeltaKind::Updated)
        .await;
    if let Err(e) = state.emit_workspace_task_upsert(task_id).await {
        tracing::warn!(task_id = %task_id.0, "workspace active snapshot refresh failed: {e:?}");
    }
    let sessions = match store.list_sessions_for_task(task_id).await {
        Ok(sessions) => sessions,
        Err(e) => {
            tracing::warn!(task_id = %task_id.0, "failed to list sessions for archived task: {e:?}");
            Vec::new()
        }
    };
    let mut worktree_ids: HashSet<WorktreeId> = sessions.iter().map(|s| s.worktree_id).collect();
    if let Some(primary_worktree_id) = task.primary_worktree_id {
        worktree_ids.insert(primary_worktree_id);
    }
    let mut worktree_id_strings = HashSet::new();
    for worktree_id in worktree_ids {
        match store.get_worktree(worktree_id).await {
            Ok(Some(worktree)) => {
                worktree_id_strings.insert(worktree.id.0.to_string());
            }
            Ok(None) => {
                tracing::warn!(
                    task_id = %task_id.0,
                    worktree_id = %worktree_id.0,
                    "worktree missing for archived task"
                );
            }
            Err(e) => {
                tracing::warn!(
                    task_id = %task_id.0,
                    worktree_id = %worktree_id.0,
                    "failed to load worktree for archived task: {e:?}"
                );
            }
        }
    }
    let session_ids: HashSet<String> = sessions
        .iter()
        .map(|session| session.id.0.to_string())
        .collect();
    if let Err(e) = state
        .transport
        .web_sessions
        .close_for_task(&session_ids, &worktree_id_strings)
        .await
    {
        tracing::warn!(task_id = %task_id.0, "failed to close web sessions for archived task: {e:?}");
    }
    Ok(Json(task))
}

pub(super) async fn delete_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = state
        .store_for_task(task_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let task = store
        .get_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let sessions = store
        .list_sessions_for_task(task_id)
        .await
        .unwrap_or_default();
    let mut worktree_ids: HashSet<WorktreeId> = sessions.iter().map(|s| s.worktree_id).collect();
    if let Some(primary_worktree_id) = task.primary_worktree_id {
        worktree_ids.insert(primary_worktree_id);
    }
    for session in &sessions {
        state.cleanup_session(session.id).await;
    }
    let deleted = store
        .delete_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !deleted {
        return Err(StatusCode::NOT_FOUND);
    }
    for worktree_id in worktree_ids {
        let other_active = match store
            .count_active_tasks_for_worktree(worktree_id, Some(task_id))
            .await
        {
            Ok(count) => count > 0,
            Err(err) => {
                tracing::warn!(
                    task_id = %task_id.0,
                    worktree_id = %worktree_id.0,
                    "failed to check worktree usage: {err:#}"
                );
                true
            }
        };
        if other_active {
            continue;
        }
        let worktree = match store.get_worktree(worktree_id).await {
            Ok(Some(worktree)) => Some(worktree),
            Ok(None) => None,
            Err(err) => {
                tracing::warn!(
                    task_id = %task_id.0,
                    worktree_id = %worktree_id.0,
                    "failed to load worktree for hooks cleanup: {err:#}"
                );
                None
            }
        };
        let worktree_root = worktree
            .as_ref()
            .map(|entry| StdPath::new(&entry.root_path));
        let vcs_kind = worktree.as_ref().and_then(|entry| entry.vcs_kind.clone());
        if let Err(err) = vcs_hooks::cleanup_worktree_hooks(
            &state.core.data_root,
            task.workspace_id,
            worktree_id,
            worktree_root,
            vcs_kind,
        )
        .await
        {
            tracing::warn!(
                task_id = %task_id.0,
                worktree_id = %worktree_id.0,
                "failed to remove vcs hooks: {err:#}"
            );
        }
    }
    let _ = state
        .global_store()
        .delete_workspace_task_index(task_id)
        .await;
    for session in sessions {
        let _ = state
            .global_store()
            .delete_workspace_session_index(session.id)
            .await;
    }
    state
        .emit_workspace_task_delete(task.workspace_id, task_id)
        .await;
    if task.archived_at.is_some() {
        state
            .emit_workspace_archived_task_delete(task.workspace_id, task_id)
            .await;
    }
    Ok(StatusCode::NO_CONTENT)
}

fn managed_worktree_root(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
) -> Option<PathBuf> {
    let root = PathBuf::from(&worktree.root_path);
    let expected = managed_worktree_path(&state.core.data_root, workspace.id, worktree.id);
    if root == expected {
        Some(root)
    } else {
        None
    }
}

pub(super) async fn remove_worktree(
    workspace_root: impl AsRef<StdPath>,
    worktree_path: impl AsRef<StdPath>,
) -> anyhow::Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_root.as_ref())
        .arg("worktree")
        .arg("remove")
        .arg("--force")
        .arg(worktree_path.as_ref())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git worktree remove")?;
    if !output.status.success() {
        bail!(
            "git worktree remove failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    if tokio::fs::metadata(worktree_path.as_ref()).await.is_ok() {
        tokio::fs::remove_dir_all(worktree_path.as_ref())
            .await
            .context("removing worktree dir")?;
    }
    Ok(())
}

pub(super) async fn prune_worktrees(workspace_root: impl AsRef<StdPath>) -> anyhow::Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_root.as_ref())
        .arg("worktree")
        .arg("prune")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git worktree prune")?;
    if !output.status.success() {
        bail!(
            "git worktree prune failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

pub(super) async fn ensure_worktree_attached(
    workspace_root: impl AsRef<StdPath>,
    worktree_path: impl AsRef<StdPath>,
    base_commit_sha: &str,
    branch_name: &str,
) -> anyhow::Result<()> {
    let worktree_path = worktree_path.as_ref();
    if let Some(parent) = worktree_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .context("creating worktree parent dir")?;
    }

    if tokio::fs::metadata(worktree_path).await.is_ok() {
        if is_git_worktree(worktree_path).await.unwrap_or(false) {
            return Ok(());
        }
        tokio::fs::remove_dir_all(worktree_path)
            .await
            .context("removing stale worktree dir")?;
    }

    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(workspace_root.as_ref())
        .arg("worktree")
        .arg("add")
        .arg(worktree_path);
    if branch_exists(workspace_root, branch_name).await? {
        cmd.arg(branch_name);
    } else {
        cmd.arg("-b").arg(branch_name).arg(base_commit_sha);
    }
    let output = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git worktree add")?;
    if !output.status.success() {
        bail!(
            "git worktree add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

pub(super) async fn branch_exists(
    workspace_root: impl AsRef<StdPath>,
    branch_name: &str,
) -> anyhow::Result<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_root.as_ref())
        .arg("show-ref")
        .arg("--verify")
        .arg("--quiet")
        .arg(format!("refs/heads/{branch_name}"))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git show-ref --verify")?;
    if output.status.success() {
        return Ok(true);
    }
    if output.status.code() == Some(1) {
        return Ok(false);
    }
    bail!(
        "git show-ref failed: {}",
        String::from_utf8_lossy(&output.stderr)
    )
}

pub(super) async fn is_git_worktree(worktree_path: impl AsRef<StdPath>) -> anyhow::Result<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(worktree_path.as_ref())
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("running git rev-parse --is-inside-work-tree")?;
    if !output.status.success() {
        return Ok(false);
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim() == "true")
}

mod snapshot_state;
pub(super) use snapshot_state::load_workspace_active_snapshot_state;

#[cfg(test)]
mod tests;
