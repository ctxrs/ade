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

use super::errors::ApiErrorResp;
use super::sessions::schedule_session_title_generation;
use super::shared::session_root_kind_for_worktree;
use crate::attachments;
use crate::daemon::AppState;
use crate::execution_effective;
use crate::logs;
use crate::ops_events::OpsEvent;
use crate::scheduler::SchedulerCommand;
use crate::settings::{ContainerMountMode, ExecutionMode, ExecutionSettings};
use crate::telemetry::TelemetryEvent;
use crate::vcs_hooks;
use crate::worktree_bootstrap;
use ctx_core::ids::{MessageId, RunId, SessionId, TaskId, TurnId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    ExecutionEnvironment, Message, MessageDelivery, MessageRole, Session, SessionEventType,
    SessionTurn, SessionTurnStatus, Task, TaskDeltaKind, VcsKind, Workspace, WorkspaceArchivedPage,
    WorkspaceIndexCursor, Worktree,
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
        ExecutionMode::Container => match settings.container.mount_mode {
            ContainerMountMode::HostMounted => ExecutionEnvironment::ContainerHostMounted,
            ContainerMountMode::DiskIsolated => ExecutionEnvironment::ContainerDiskIsolated,
        },
    }
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

#[derive(Debug, Serialize)]
pub(super) struct ArchiveTaskResponse {
    #[serde(flatten)]
    task: Task,
    cleanup_failed: bool,
}

pub(super) async fn archive_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ArchiveTaskResponse>, StatusCode> {
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
    let workspace = state
        .global_store()
        .get_workspace(task.workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let sessions = store
        .list_sessions_for_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    for session in &sessions {
        state.cleanup_session(session.id).await;
    }
    let mut worktree_ids: HashSet<WorktreeId> = sessions.iter().map(|s| s.worktree_id).collect();
    if let Some(primary_worktree_id) = task.primary_worktree_id {
        worktree_ids.insert(primary_worktree_id);
    }
    let mut seen = HashSet::new();
    let mut worktrees = Vec::new();
    for worktree_id in worktree_ids {
        if !seen.insert(worktree_id) {
            continue;
        }
        let worktree = store
            .get_worktree(worktree_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        worktrees.push(worktree);
    }

    let mut errors: Vec<anyhow::Error> = Vec::new();
    let mut needs_prune = false;
    for worktree in &worktrees {
        let other_active = match store
            .count_active_tasks_for_worktree(worktree.id, Some(task_id))
            .await
        {
            Ok(count) => count > 0,
            Err(err) => {
                tracing::warn!(
                    task_id = %task_id.0,
                    worktree_id = %worktree.id.0,
                    "failed to check worktree usage: {err:#}"
                );
                true
            }
        };
        if other_active {
            continue;
        }
        if let Err(err) = vcs_hooks::cleanup_worktree_hooks(
            &state.core.data_root,
            workspace.id,
            worktree.id,
            Some(StdPath::new(&worktree.root_path)),
            worktree.vcs_kind.clone(),
        )
        .await
        {
            tracing::warn!(
                task_id = %task_id.0,
                worktree_id = %worktree.id.0,
                "failed to remove vcs hooks: {err:#}"
            );
        }
        let Some(root) = managed_worktree_root(&state, &workspace, worktree) else {
            continue;
        };
        let branch = worktree
            .git_branch
            .as_deref()
            .filter(|name| name.starts_with("ctx/"));
        if tokio::fs::metadata(&root).await.is_err() {
            if branch.is_some() {
                needs_prune = true;
            }
            if let Some(branch) = branch {
                if let Err(err) = delete_branch(&workspace.root_path, branch).await {
                    tracing::warn!(
                        task_id = %task_id.0,
                        worktree_id = %worktree.id.0,
                        branch,
                        "failed to delete worktree branch: {err:#}"
                    );
                }
            }
            continue;
        }
        let is_git = is_git_worktree(&root).await.unwrap_or(false);
        if is_git {
            needs_prune = true;
            if let Err(err) = remove_worktree(&workspace.root_path, &root).await {
                tracing::warn!(
                    task_id = %task_id.0,
                    worktree_id = %worktree.id.0,
                    "failed to remove worktree: {err:#}"
                );
                errors.push(err);
                continue;
            }
            // Defensive: ensure the directory is actually gone even if `git worktree remove`
            // succeeds but leaves the directory behind.
            if tokio::fs::metadata(&root).await.is_ok() {
                if let Err(err) = tokio::fs::remove_dir_all(&root)
                    .await
                    .with_context(|| format!("removing worktree dir at {}", root.display()))
                {
                    tracing::warn!(
                        task_id = %task_id.0,
                        worktree_id = %worktree.id.0,
                        "failed to remove worktree dir: {err:#}"
                    );
                    errors.push(err);
                }
            }
        } else if let Err(err) = tokio::fs::remove_dir_all(&root)
            .await
            .with_context(|| format!("removing non-git worktree dir at {}", root.display()))
        {
            tracing::warn!(
                task_id = %task_id.0,
                worktree_id = %worktree.id.0,
                "failed to remove worktree dir: {err:#}"
            );
            errors.push(err);
        }
        if let Some(branch) = branch {
            if let Err(err) = delete_branch(&workspace.root_path, branch).await {
                tracing::warn!(
                    task_id = %task_id.0,
                    worktree_id = %worktree.id.0,
                    branch,
                    "failed to delete worktree branch: {err:#}"
                );
            }
        }
    }
    if needs_prune {
        if let Err(err) = prune_worktrees(&workspace.root_path).await {
            tracing::warn!(
                task_id = %task_id.0,
                "failed to prune worktrees: {err:#}"
            );
            errors.push(err);
        }
    }
    let cleanup_failed = !errors.is_empty();
    if cleanup_failed {
        tracing::warn!(task_id = %task_id.0, "archive cleanup had errors; task will still be archived");
    }

    let updated = store
        .archive_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !updated {
        return Err(StatusCode::NOT_FOUND);
    }
    let task = match store
        .get_task_with_activity(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        Some(task) => task,
        None => return Err(StatusCode::NOT_FOUND),
    };
    let _ = state
        .emit_workspace_task_delta(task.clone(), TaskDeltaKind::Archived)
        .await;
    if let Err(e) = state.emit_workspace_task_upsert(task_id).await {
        tracing::warn!(task_id = %task_id.0, "workspace active snapshot refresh failed: {e:?}");
    }
    Ok(Json(ArchiveTaskResponse {
        task,
        cleanup_failed,
    }))
}

pub(super) async fn unarchive_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Task>, StatusCode> {
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
    let workspace = state
        .global_store()
        .get_workspace(task.workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let mut seen = HashSet::new();
    let mut managed_worktrees: Vec<(Worktree, PathBuf)> = Vec::new();
    let mut worktrees: Vec<Worktree> = Vec::new();
    let sessions = store
        .list_sessions_for_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut worktree_ids: HashSet<WorktreeId> = sessions.iter().map(|s| s.worktree_id).collect();
    if let Some(primary) = task.primary_worktree_id {
        worktree_ids.insert(primary);
    }
    for worktree_id in worktree_ids {
        let worktree = store
            .get_worktree(worktree_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        if let Some(root) = managed_worktree_root(&state, &workspace, &worktree) {
            if seen.insert(worktree.id) {
                managed_worktrees.push((worktree.clone(), root));
            }
        }
        worktrees.push(worktree);
    }

    for (worktree, root) in &managed_worktrees {
        let branch = worktree.git_branch.as_deref().unwrap_or_default();
        if let Err(err) = ensure_worktree_attached(
            &workspace.root_path,
            root,
            &worktree.base_commit_sha,
            branch,
        )
        .await
        {
            tracing::warn!(
                task_id = %task_id.0,
                worktree_id = %worktree.id.0,
                "failed to recreate worktree: {err:#}"
            );
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    for worktree in &worktrees {
        if let Err(e) = attachments::ensure_worktree_attachment_mounts_if_materialized(
            &state, &workspace, worktree,
        )
        .await
        {
            tracing::warn!(task_id = %task_id.0, "attachment mounts failed: {e:?}");
        }
        if let Err(e) = worktree_bootstrap::spawn_worktree_bootstrap(
            Arc::clone(&state),
            workspace.clone(),
            worktree.clone(),
        )
        .await
        {
            tracing::warn!(task_id = %task_id.0, "worktree bootstrap failed: {e:?}");
        }
        if let Err(e) = vcs_hooks::ensure_task_commit_hook(
            &state.core.data_root,
            workspace.id,
            worktree.id,
            StdPath::new(&worktree.root_path),
            worktree.vcs_kind.clone(),
            task_id,
        )
        .await
        {
            tracing::warn!(
                task_id = %task_id.0,
                worktree_id = %worktree.id.0,
                "failed to configure vcs hooks: {e:#}"
            );
        }
    }

    let updated = store
        .unarchive_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !updated {
        return Err(StatusCode::NOT_FOUND);
    }
    let task = match store
        .get_task_with_activity(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        Some(task) => task,
        None => return Err(StatusCode::NOT_FOUND),
    };
    let _ = state
        .emit_workspace_task_delta(task.clone(), TaskDeltaKind::Unarchived)
        .await;
    if let Err(e) = state.emit_workspace_task_upsert(task_id).await {
        tracing::warn!(task_id = %task_id.0, "workspace active snapshot refresh failed: {e:?}");
    }
    state
        .emit_workspace_archived_task_delete(task.workspace_id, task_id)
        .await;
    Ok(Json(task))
}

pub(super) async fn mark_task_read(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Task>, StatusCode> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = state
        .store_for_task(task_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let updated = store
        .mark_task_read(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !updated {
        return Err(StatusCode::NOT_FOUND);
    }
    let task = match store
        .get_task_with_activity(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        Some(task) => task,
        None => return Err(StatusCode::NOT_FOUND),
    };
    let _ = state
        .emit_workspace_task_delta(task.clone(), TaskDeltaKind::Updated)
        .await;
    if let Err(e) = state.emit_workspace_task_upsert(task_id).await {
        tracing::warn!(task_id = %task_id.0, "workspace active snapshot refresh failed: {e:?}");
    }
    Ok(Json(task))
}

pub(super) async fn mark_task_unread(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Task>, StatusCode> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = state
        .store_for_task(task_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let updated = store
        .mark_task_unread(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !updated {
        return Err(StatusCode::NOT_FOUND);
    }
    let task = match store
        .get_task_with_activity(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        Some(task) => task,
        None => return Err(StatusCode::NOT_FOUND),
    };
    let _ = state
        .emit_workspace_task_delta(task.clone(), TaskDeltaKind::Updated)
        .await;
    if let Err(e) = state.emit_workspace_task_upsert(task_id).await {
        tracing::warn!(task_id = %task_id.0, "workspace active snapshot refresh failed: {e:?}");
    }
    Ok(Json(task))
}

pub(super) async fn list_workspace_tasks(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<Task>>, StatusCode> {
    let workspace_id =
        WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = state
        .store_for_workspace(workspace_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let tasks = store
        .list_tasks(workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(tasks))
}

#[derive(Debug, Deserialize)]
pub(super) struct WorkspaceArchivedQuery {
    limit: Option<u32>,
    cursor_sort_at: Option<String>,
    cursor_task_id: Option<String>,
}

pub(super) async fn list_workspace_archived_task_summaries(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<WorkspaceArchivedQuery>,
) -> Result<Json<WorkspaceArchivedPage>, StatusCode> {
    let workspace_id =
        WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let limit = query.limit.unwrap_or(50) as i64;
    let cursor = match (
        query.cursor_sort_at.as_deref(),
        query.cursor_task_id.as_deref(),
    ) {
        (None, None) => None,
        (Some(sort_at), Some(task_id)) => {
            let sort_at = DateTime::parse_from_rfc3339(sort_at)
                .map_err(|_| StatusCode::BAD_REQUEST)?
                .with_timezone(&Utc);
            let task_id =
                TaskId(uuid::Uuid::parse_str(task_id).map_err(|_| StatusCode::BAD_REQUEST)?);
            Some(WorkspaceIndexCursor { sort_at, task_id })
        }
        _ => return Err(StatusCode::BAD_REQUEST),
    };

    let store = state
        .store_for_workspace(workspace_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let (tasks, next_cursor) = store
        .list_workspace_archived_page(workspace_id, cursor, limit)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let (_, total_archived) = store
        .workspace_task_counts(workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let (_, archived_rev) = load_workspace_active_snapshot_state(&state, workspace_id).await;

    Ok(Json(WorkspaceArchivedPage {
        workspace_id,
        archived_rev,
        tasks,
        next_cursor,
        total_archived,
    }))
}

pub(super) async fn list_task_sessions(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<Session>>, StatusCode> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = state
        .store_for_task(task_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let sessions = store
        .list_sessions_for_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(sessions))
}

pub(super) async fn create_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<CreateTaskReq>,
) -> Result<Json<Task>, (StatusCode, Json<ApiErrorResp>)> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    let ws = state
        .global_store()
        .get_workspace(ws_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;

    let store = state.store_for_workspace(ws_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;

    let task_id = match req.id.as_deref().map(str::trim) {
        Some("") | None => None,
        Some(raw) => Some(TaskId(uuid::Uuid::parse_str(raw).map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "invalid task id".to_string(),
                }),
            )
        })?)),
    };
    if let Some(task_id) = task_id {
        let existing_ws = state
            .global_store()
            .get_workspace_id_for_task(task_id)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;
        if let Some(existing_ws) = existing_ws {
            if existing_ws != ws_id {
                return Err((
                    StatusCode::CONFLICT,
                    Json(ApiErrorResp {
                        error: "task id already exists".to_string(),
                    }),
                ));
            }
            let existing = store.get_task(task_id).await.map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;
            if let Some(existing) = existing {
                if !task_request_matches(&existing, &req.title, &req.description) {
                    return Err((
                        StatusCode::CONFLICT,
                        Json(ApiErrorResp {
                            error: "task id already exists".to_string(),
                        }),
                    ));
                }
                return Ok(Json(existing));
            }
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "task index exists but task missing".to_string(),
                }),
            ));
        }
    }

    let want_default_session = req.create_default_session;
    let ws_root = StdPath::new(&ws.root_path);
    let vcs = if want_default_session {
        let vcs = vcs::driver_for_path(ws_root).await.map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
        vcs.assert_repo(ws_root).await.map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
        Some(vcs)
    } else {
        None
    };

    let requested_title = req.title.clone();
    let requested_description = req.description.clone();
    let task = match task_id {
        Some(task_id) => {
            store
                .create_task_with_id(ws_id, task_id, req.title, req.description)
                .await
        }
        None => store.create_task(ws_id, req.title, req.description).await,
    }
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    if task_id.is_some() && task.workspace_id != ws_id {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "task id already exists".to_string(),
            }),
        ));
    }
    if task_id.is_some() && !task_request_matches(&task, &requested_title, &requested_description) {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "task id already exists".to_string(),
            }),
        ));
    }
    if let Err(e) = state
        .global_store()
        .upsert_workspace_task_index(task.id, ws_id)
        .await
    {
        tracing::warn!(task_id = %task.id.0, "failed to update task index: {e:?}");
    }

    if !req.create_default_session {
        if let Err(e) = state.emit_workspace_task_upsert(task.id).await {
            tracing::warn!(task_id = %task.id.0, "workspace active snapshot refresh failed: {e:?}");
        }
        return Ok(Json(task));
    }

    let Some(vcs) = vcs else {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: "missing vcs driver for default session".to_string(),
            }),
        ));
    };
    let base_commit_sha = vcs.rev_parse_head(ws_root).await.map_err(|e| {
        let msg = e.to_string().to_lowercase();
        if msg.contains("ambiguous argument 'head'")
            || msg.contains("unknown revision or path not in the working tree")
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "git repo has no commits; create an initial commit before creating a worktree".to_string(),
                }),
            );
        }
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;

    let worktree_id = WorktreeId::new();
    let branch_name = format!("ctx/{}/{}", task.id.0, worktree_id.0);
    let effective = execution_effective::effective_execution_settings(&state, ws.id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let wt_path = if matches!(effective.mode, ExecutionMode::Container)
        && matches!(
            effective.container.mount_mode,
            ContainerMountMode::DiskIsolated
        ) {
        // Ensure the harness container exists (disk-isolated worktrees live inside it).
        if let Err(e) = state
            .execution
            .harness
            .ensure_workspace_container(&ws, &effective, &state.core.daemon_url)
            .await
        {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            ));
        }
        crate::disk_isolated::ensure_worktree_from_host_copy(
            &state.core.data_root,
            ws_id,
            worktree_id,
            ws_root,
            &base_commit_sha,
            &branch_name,
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: format!(
                        "disk-isolated worktree provisioning failed: {}. \
retry after checking container runtime health.",
                        logs::redact_sensitive(&e.to_string())
                    ),
                }),
            )
        })?
    } else {
        let wt_path = managed_worktree_path(&state.core.data_root, ws_id, worktree_id);
        if let Some(parent) = wt_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;
        }
        create_worktree(&ws.root_path, &wt_path, &base_commit_sha, &branch_name)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;
        wt_path
    };

    let worktree = Worktree {
        id: worktree_id,
        workspace_id: ws_id,
        root_path: wt_path.to_string_lossy().to_string(),
        base_commit_sha: base_commit_sha.clone(),
        git_branch: (vcs.kind() == VcsKind::Git).then(|| branch_name.clone()),
        vcs_kind: Some(vcs.kind()),
        base_revision: Some(base_commit_sha.clone()),
        vcs_ref: Some(branch_name.clone()),
        created_at: chrono::Utc::now(),
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
    store.insert_worktree(worktree.clone()).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    if let Err(e) = state
        .global_store()
        .upsert_workspace_worktree_index(worktree_id, ws_id)
        .await
    {
        tracing::warn!(worktree_id = %worktree_id.0, "failed to update worktree index: {e:?}");
    }

    if let Err(e) = vcs_hooks::ensure_task_commit_hook(
        &state.core.data_root,
        ws_id,
        worktree_id,
        StdPath::new(&worktree.root_path),
        worktree.vcs_kind.clone(),
        task.id,
    )
    .await
    {
        tracing::warn!(
            task_id = %task.id.0,
            worktree_id = %worktree_id.0,
            "failed to configure vcs hooks: {e:#}"
        );
    }

    if let Err(e) = worktree_bootstrap::spawn_worktree_bootstrap(
        Arc::clone(&state),
        ws.clone(),
        worktree.clone(),
    )
    .await
    {
        tracing::warn!(task_id = %task.id.0, "worktree bootstrap failed: {e:?}");
    }

    if let Err(e) = store.set_task_primary_worktree(task.id, worktree_id).await {
        tracing::warn!(task_id = %task.id.0, "failed to set primary worktree: {e:?}");
    }

    if let Err(e) = attachments::sync_workspace_attachments(Arc::clone(&state), &ws, false).await {
        tracing::warn!(task_id = %task.id.0, "attachment sync failed: {e:?}");
    }

    if let Err(e) =
        attachments::ensure_worktree_attachment_mounts_if_materialized(&state, &ws, &worktree).await
    {
        tracing::warn!(task_id = %task.id.0, "attachment mounts failed: {e:?}");
    }

    let task = match store.get_task_with_activity(task.id).await.map_err(|e| {
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

    if let Err(e) = state.emit_workspace_task_upsert(task.id).await {
        tracing::warn!(task_id = %task.id.0, "workspace active snapshot refresh failed: {e:?}");
    }
    Ok(Json(task))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateSessionReq {
    #[serde(default)]
    id: Option<String>,
    provider_id: String,
    model_id: String,
    parent_session_id: Option<String>,
    relationship: Option<String>,
    #[serde(default)]
    initial_prompt: Option<String>,
    #[serde(default)]
    initial_message_id: Option<String>,
    #[serde(default)]
    initial_turn_id: Option<String>,
    #[serde(default)]
    worktree_id: Option<String>,
    #[serde(default)]
    execution_environment: Option<ExecutionEnvironment>,
}

pub(super) async fn create_session_for_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<CreateSessionReq>,
) -> Result<Json<Session>, StatusCode> {
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
    let workspace = state
        .global_store()
        .get_workspace(task.workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let run_id_header = headers
        .get("x-ctx-run-id")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string());
    let provider_id = req.provider_id.clone();
    let model_id = req.model_id.clone();

    let session_id = match req.id.as_deref().map(str::trim) {
        Some("") | None => None,
        Some(raw) => Some(SessionId(
            uuid::Uuid::parse_str(raw).map_err(|_| StatusCode::BAD_REQUEST)?,
        )),
    };
    let parent_session_id = match req.parent_session_id {
        Some(id) => Some(SessionId(
            uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?,
        )),
        None => None,
    };
    let relationship = req
        .relationship
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());
    let effective = execution_effective::effective_execution_settings(&state, workspace.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let effective_execution_environment = execution_environment_from_settings(&effective);
    let execution_environment = match req.execution_environment {
        Some(requested) => {
            if requested != effective_execution_environment {
                return Err(StatusCode::BAD_REQUEST);
            }
            requested
        }
        None => effective_execution_environment,
    };
    let requested_relationship = relationship.clone();
    if parent_session_id.is_some() != relationship.is_some() {
        return Err(StatusCode::BAD_REQUEST);
    }
    if req.initial_prompt.is_some()
        && (req.initial_message_id.is_none() || req.initial_turn_id.is_none())
    {
        state
            .emit_compat_payload_reject_counter("tasks.create_session", "missing_initial_ids", None)
            .await;
        return Err(StatusCode::BAD_REQUEST);
    }

    let worktree_id = if let Some(worktree_id) = req.worktree_id.as_deref() {
        WorktreeId(uuid::Uuid::parse_str(worktree_id).map_err(|_| StatusCode::BAD_REQUEST)?)
    } else if let Some(primary) = task.primary_worktree_id {
        primary
    } else {
        let workspace_root = StdPath::new(&workspace.root_path);
        let vcs = vcs::driver_for_path(workspace_root)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let base_commit_sha = vcs.rev_parse_head(workspace_root).await.map_err(|e| {
            let msg = e.to_string().to_lowercase();
            if msg.contains("ambiguous argument 'head'")
                || msg.contains("unknown revision or path not in the working tree")
            {
                return StatusCode::BAD_REQUEST;
            }
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
        let worktree_id = WorktreeId::new();
        let branch_name = format!("ctx/{}/{}", task.id.0, worktree_id.0);
        let wt_path = if matches!(effective.mode, ExecutionMode::Container)
            && matches!(
                effective.container.mount_mode,
                ContainerMountMode::DiskIsolated
            ) {
            state
                .execution
                .harness
                .ensure_workspace_container(&workspace, &effective, &state.core.daemon_url)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            crate::disk_isolated::ensure_worktree_from_host_copy(
                &state.core.data_root,
                task.workspace_id,
                worktree_id,
                workspace_root,
                &base_commit_sha,
                &branch_name,
            )
            .await
            .map_err(|e| {
                tracing::warn!(
                    task_id = %task.id.0,
                    worktree_id = %worktree_id.0,
                    "disk-isolated worktree provisioning failed: {e:#}"
                );
                StatusCode::INTERNAL_SERVER_ERROR
            })?
        } else {
            let wt_path =
                managed_worktree_path(&state.core.data_root, task.workspace_id, worktree_id);
            if let Some(parent) = wt_path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            }
            create_worktree(
                &workspace.root_path,
                &wt_path,
                &base_commit_sha,
                &branch_name,
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            wt_path
        };

        let worktree = Worktree {
            id: worktree_id,
            workspace_id: task.workspace_id,
            root_path: wt_path.to_string_lossy().to_string(),
            base_commit_sha: base_commit_sha.clone(),
            git_branch: (vcs.kind() == VcsKind::Git).then(|| branch_name.clone()),
            vcs_kind: Some(vcs.kind()),
            base_revision: Some(base_commit_sha.clone()),
            vcs_ref: Some(branch_name.clone()),
            created_at: chrono::Utc::now(),
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
        store
            .insert_worktree(worktree.clone())
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if let Err(e) = retry_global_index_write(|| async {
            state
                .global_store()
                .upsert_workspace_worktree_index(worktree_id, task.workspace_id)
                .await
        })
        .await
        {
            tracing::warn!(
                worktree_id = %worktree_id.0,
                "failed to update worktree index: {e:?}"
            );
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
        if let Err(e) = worktree_bootstrap::spawn_worktree_bootstrap(
            Arc::clone(&state),
            workspace.clone(),
            worktree.clone(),
        )
        .await
        {
            tracing::warn!(task_id = %task.id.0, "worktree bootstrap failed: {e:?}");
        }
        if let Err(e) =
            attachments::sync_workspace_attachments(Arc::clone(&state), &workspace, false).await
        {
            tracing::warn!(task_id = %task.id.0, "attachment sync failed: {e:?}");
        }
        if let Err(e) = attachments::ensure_worktree_attachment_mounts_if_materialized(
            &state, &workspace, &worktree,
        )
        .await
        {
            tracing::warn!(task_id = %task.id.0, "attachment mounts failed: {e:?}");
        }
        worktree_id
    };

    if let Some(session_id) = session_id {
        let existing_ws = state
            .global_store()
            .get_workspace_id_for_session(session_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if let Some(existing_ws) = existing_ws {
            if existing_ws != task.workspace_id {
                return Err(StatusCode::CONFLICT);
            }
            let existing = store
                .get_session(session_id)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            if let Some(existing) = existing {
                if existing.task_id != task_id
                    || existing.workspace_id != task.workspace_id
                    || existing.worktree_id != worktree_id
                    || existing.execution_environment != execution_environment
                    || existing.provider_id != provider_id
                    || existing.model_id != model_id
                    || existing.parent_session_id != parent_session_id
                    || existing.relationship != relationship
                {
                    return Err(StatusCode::CONFLICT);
                }
                state.remember_session_meta(&existing).await;
                return Ok(Json(existing));
            }
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    if let Ok(Some(worktree)) = store.get_worktree(worktree_id).await {
        if let Err(e) = vcs_hooks::ensure_task_commit_hook(
            &state.core.data_root,
            task.workspace_id,
            worktree.id,
            StdPath::new(&worktree.root_path),
            worktree.vcs_kind.clone(),
            task.id,
        )
        .await
        {
            tracing::warn!(
                task_id = %task.id.0,
                worktree_id = %worktree.id.0,
                "failed to configure vcs hooks: {e:#}"
            );
        }
    }

    let requested_session_id = session_id;
    let session = if let Some(session_id) = requested_session_id {
        store
            .create_session_with_id(
                session_id,
                task_id,
                task.workspace_id,
                worktree_id,
                execution_environment,
                provider_id.clone(),
                model_id.clone(),
                "implementer".to_string(),
                parent_session_id,
                relationship.clone(),
                None,
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    } else {
        store
            .create_session(
                task_id,
                task.workspace_id,
                worktree_id,
                execution_environment,
                provider_id.clone(),
                model_id.clone(),
                "implementer".to_string(),
                parent_session_id,
                relationship.clone(),
                None,
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    };
    if let Some(session_id) = requested_session_id {
        if session.id != session_id
            || session.task_id != task_id
            || session.workspace_id != task.workspace_id
            || session.worktree_id != worktree_id
            || session.execution_environment != execution_environment
            || session.provider_id != provider_id
            || session.model_id != model_id
            || session.parent_session_id != parent_session_id
            || session.relationship != requested_relationship
        {
            return Err(StatusCode::CONFLICT);
        }
    }
    state.remember_session_meta(&session).await;
    if let Err(e) = retry_global_index_write(|| async {
        state
            .global_store()
            .upsert_workspace_session_index(session.id, task.workspace_id)
            .await
    })
    .await
    {
        tracing::warn!(session_id = %session.id.0, "failed to update session index: {e:?}");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    if session.parent_session_id.is_none() && session.relationship.is_none() {
        let _ = store
            .set_task_primary_session(task.id, session.id, worktree_id)
            .await;
    }

    if let Some(prompt) = req.initial_prompt {
        let (message_id, turn_id) = match (
            req.initial_message_id.as_deref(),
            req.initial_turn_id.as_deref(),
        ) {
            (Some(message_id), Some(turn_id)) => (
                MessageId(uuid::Uuid::parse_str(message_id).map_err(|_| StatusCode::BAD_REQUEST)?),
                TurnId(uuid::Uuid::parse_str(turn_id).map_err(|_| StatusCode::BAD_REQUEST)?),
            ),
            _ => return Err(StatusCode::BAD_REQUEST),
        };

        let delivery = MessageDelivery::Immediate;
        let attachments = Vec::new();
        if let Some(existing) = store
            .get_message(message_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        {
            let matches = existing.session_id == session.id
                && existing.turn_id == Some(turn_id)
                && matches!(existing.role, MessageRole::User)
                && existing.content == prompt
                && existing.attachments.is_empty()
                && matches!(existing.delivery, MessageDelivery::Immediate);
            if matches {
                super::sessions::ensure_session_turn_for_message(
                    &store, session.id, turn_id, &existing,
                )
                .await?;
            } else {
                return Err(StatusCode::CONFLICT);
            }
        } else {
            // Fall through to create below.
            let prompt_for_idempotency = prompt.clone();

            let run_id = RunId::new();
            let order_seq_state = state.sessions.get_order_seq_state(&store, session.id).await;
            let order_seq = {
                let mut order_seq_state = order_seq_state.lock().await;
                order_seq_state.get_or_assign(format!("message:{}", message_id.0), None)
            };
            let msg = Message {
                id: message_id,
                session_id: session.id,
                task_id: session.task_id,
                run_id: Some(run_id),
                turn_id: Some(turn_id),
                turn_sequence: Some(0),
                order_seq: Some(order_seq),
                role: MessageRole::User,
                content: prompt,
                attachments: attachments.clone(),
                delivery,
                delivered_at: None,
                created_at: chrono::Utc::now(),
            };

            let saved = match store.insert_message(msg).await {
                Ok(saved) => saved,
                Err(err) if is_unique_constraint_violation(&err) => {
                    let Some(existing) = store
                        .get_message(message_id)
                        .await
                        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
                    else {
                        return Err(StatusCode::INTERNAL_SERVER_ERROR);
                    };
                    let matches = existing.session_id == session.id
                        && existing.turn_id == Some(turn_id)
                        && matches!(existing.role, MessageRole::User)
                        && existing.content == prompt_for_idempotency
                        && existing.attachments.is_empty()
                        && matches!(existing.delivery, MessageDelivery::Immediate);
                    if matches {
                        state
                            .global_store()
                            .upsert_workspace_message_index(existing.id, session.workspace_id)
                            .await
                            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                        existing
                    } else {
                        return Err(StatusCode::CONFLICT);
                    }
                }
                Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
            };
            state
                .global_store()
                .upsert_workspace_message_index(saved.id, session.workspace_id)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

            let event = store
                .append_session_event(
                    session.id,
                    Some(run_id),
                    Some(turn_id),
                    SessionEventType::UserMessage,
                    serde_json::json!({
                        "message_id": saved.id.0,
                        "content": saved.content.clone(),
                        "delivery": saved.delivery.clone(),
                        "attachments": saved.attachments,
                        "order_seq": order_seq,
                    }),
                )
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            let start_seq = event.seq;

            let turn = SessionTurn {
                turn_id,
                session_id: session.id,
                run_id: Some(run_id),
                user_message_id: Some(saved.id),
                status: SessionTurnStatus::Running,
                start_seq: Some(start_seq),
                end_seq: None,
                started_at: saved.created_at,
                updated_at: saved.created_at,
                assistant_partial: None,
                thought_partial: None,
                metrics_json: None,
                tool_total: 0,
                tool_pending: 0,
                tool_running: 0,
                tool_completed: 0,
                tool_failed: 0,
            };

            if let Err(err) = store.insert_session_turn(turn).await {
                if !is_unique_constraint_violation(&err) {
                    return Err(StatusCode::INTERNAL_SERVER_ERROR);
                }
                let existing = store
                    .get_session_turn_by_id(turn_id)
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                if let Some(existing) = existing {
                    let matches = existing.session_id == session.id
                        && existing.user_message_id == Some(saved.id);
                    if !matches {
                        return Err(StatusCode::CONFLICT);
                    }
                } else {
                    return Err(StatusCode::INTERNAL_SERVER_ERROR);
                }
            }

            state.publish_event(event).await;

            let prompt = saved.content.clone();
            let tx = state.ensure_scheduler(session.clone()).await;
            let queued = crate::scheduler::QueuedMessage {
                message: saved,
                enqueued_at: Instant::now(),
                run_id: run_id_header.clone(),
            };
            let _ = tx.send(SchedulerCommand::Enqueue(queued)).await;

            let _ =
                schedule_session_title_generation(state.clone(), session.clone(), prompt, false)
                    .await;
        }
    }

    let worktree = match state.store_for_session(session.id).await {
        Ok(store) => store.get_worktree(session.worktree_id).await.ok().flatten(),
        Err(_) => None,
    };
    let session_root_kind = session_root_kind_for_worktree(worktree.as_ref()).to_string();
    state
        .telemetry
        .telemetry
        .emit(TelemetryEvent::session_started(
            session.provider_id.clone(),
            session.model_id.clone(),
            Some(session.execution_environment.as_str().to_string()),
            Some(session_root_kind.clone()),
        ))
        .await;
    let mut ops_event = OpsEvent::new("info", "session_started");
    ops_event.session_id = Some(session.id.0.to_string());
    ops_event.worktree_id = Some(session.worktree_id.0.to_string());
    ops_event.provider_id = Some(session.provider_id.clone());
    ops_event.meta = Some(serde_json::json!({
        "model_id": session.model_id.clone(),
        "execution_environment": session.execution_environment.as_str(),
        "session_root_kind": session_root_kind.clone(),
        "parent_session_id": session.parent_session_id.map(|id| id.0.to_string()),
        "relationship": session.relationship.clone(),
    }));
    state.telemetry.ops_events.emit(ops_event);
    if let Err(e) = state.emit_workspace_task_upsert(session.task_id).await {
        tracing::warn!(task_id = %session.task_id.0, "workspace active snapshot refresh failed: {e:?}");
    }

    Ok(Json(session))
}

mod snapshot_state;
pub(super) use snapshot_state::load_workspace_active_snapshot_state;

#[cfg(test)]
mod tests;
