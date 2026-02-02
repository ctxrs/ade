use std::collections::HashSet;
use std::path::{Path as StdPath, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Context};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use super::errors::ApiErrorResp;
use super::sessions::schedule_session_title_generation;
use super::shared::{env_target_for_worktree, SessionWithEnv};
use crate::attachments;
use crate::daemon::AppState;
use crate::logs;
use crate::ops_events::OpsEvent;
use crate::scheduler::SchedulerCommand;
use crate::telemetry::TelemetryEvent;
use crate::vcs_hooks;
use crate::worktree_bootstrap;
use ctx_core::ids::{MessageId, RunId, SessionId, TaskId, TurnId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    Message, MessageDelivery, MessageRole, Session, SessionEventType, SessionTurn,
    SessionTurnStatus, Task, VcsKind, Workspace, WorkspaceArchivedPage, WorkspaceIndexCursor,
    Worktree,
};
use ctx_fs::git::delete_branch;
use ctx_fs::vcs;
use ctx_fs::worktrees::{create_worktree, managed_worktree_path};

#[derive(Debug, Deserialize)]
pub(super) struct UpdateTaskTitleReq {
    title: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct CreateTaskReq {
    title: String,
    description: Option<String>,
    #[serde(default = "default_true")]
    create_default_session: bool,
}

fn default_true() -> bool {
    true
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

    let task = store
        .create_task(ws_id, req.title, req.description)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
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

    let vcs = vcs.expect("vcs is set for default session");
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
    let branch_name = format!("ctx/{}/{}", task.id.0, worktree_id.0);
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
        bootstrap_config_path: None,
        bootstrap_config_key: None,
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
pub(super) struct CreateSessionReq {
    provider_id: String,
    model_id: String,
    parent_session_id: Option<String>,
    relationship: Option<String>,
    #[allow(dead_code)]
    initial_prompt: Option<String>,
    #[serde(default)]
    worktree_id: Option<String>,
    #[serde(default)]
    env_target: Option<String>, // "worktree" | "local"
}

pub(super) async fn create_session_for_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<CreateSessionReq>,
) -> Result<Json<SessionWithEnv>, StatusCode> {
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
    if parent_session_id.is_some() != relationship.is_some() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let worktree_id = if let Some(worktree_id) = req.worktree_id.as_deref() {
        WorktreeId(uuid::Uuid::parse_str(worktree_id).map_err(|_| StatusCode::BAD_REQUEST)?)
    } else if let Some(primary) = task.primary_worktree_id {
        primary
    } else {
        let env_target = req
            .env_target
            .as_deref()
            .unwrap_or("local")
            .trim()
            .to_lowercase();
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
        match env_target.as_str() {
            "worktree" | "cloud" => {
                let worktree_id = WorktreeId::new();
                let wt_path =
                    managed_worktree_path(&state.core.data_root, task.workspace_id, worktree_id);
                if let Some(parent) = wt_path.parent() {
                    tokio::fs::create_dir_all(parent)
                        .await
                        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                }
                let branch_name = format!("ctx/{}/{}", task.id.0, worktree_id.0);
                create_worktree(
                    &workspace.root_path,
                    &wt_path,
                    &base_commit_sha,
                    &branch_name,
                )
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

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
                    bootstrap_config_path: None,
                    bootstrap_config_key: None,
                    bootstrap_command: None,
                    bootstrap_script_path: None,
                };
                store
                    .insert_worktree(worktree.clone())
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                if let Err(e) = state
                    .global_store()
                    .upsert_workspace_worktree_index(worktree_id, task.workspace_id)
                    .await
                {
                    tracing::warn!(
                        worktree_id = %worktree_id.0,
                        "failed to update worktree index: {e:?}"
                    );
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
                    attachments::sync_workspace_attachments(Arc::clone(&state), &workspace, false)
                        .await
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
            }
            _ => return Err(StatusCode::BAD_REQUEST),
        }
    };

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

    let session = store
        .create_session(
            task_id,
            task.workspace_id,
            worktree_id,
            req.provider_id,
            req.model_id,
            "implementer".to_string(),
            parent_session_id,
            relationship,
            None,
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state.remember_session_meta(&session).await;
    if let Err(e) = state
        .global_store()
        .upsert_workspace_session_index(session.id, task.workspace_id)
        .await
    {
        tracing::warn!(session_id = %session.id.0, "failed to update session index: {e:?}");
    }

    if session.parent_session_id.is_none() && session.relationship.is_none() {
        let _ = store
            .set_task_primary_session(task.id, session.id, worktree_id)
            .await;
    }

    if let Some(prompt) = req.initial_prompt {
        let run_id = RunId::new();
        let turn_id = TurnId::new();
        let msg = Message {
            id: MessageId::new(),
            session_id: session.id,
            task_id: session.task_id,
            run_id: Some(run_id),
            turn_id: Some(turn_id),
            turn_sequence: Some(0),
            role: MessageRole::User,
            content: prompt,
            attachments: vec![],
            delivery: MessageDelivery::Immediate,
            delivered_at: None,
            created_at: chrono::Utc::now(),
        };
        let saved = store
            .insert_message(msg)
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
        let _ = store.insert_session_turn(turn).await;
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
            schedule_session_title_generation(state.clone(), session.clone(), prompt, false).await;
    }

    let worktree = match state.store_for_session(session.id).await {
        Ok(store) => store.get_worktree(session.worktree_id).await.ok().flatten(),
        Err(_) => None,
    };
    let env_target = env_target_for_worktree(worktree.as_ref());
    state
        .telemetry
        .telemetry
        .emit(TelemetryEvent::session_started(
            session.provider_id.clone(),
            session.model_id.clone(),
            Some(env_target.clone()),
        ))
        .await;
    let mut ops_event = OpsEvent::new("info", "session_started");
    ops_event.session_id = Some(session.id.0.to_string());
    ops_event.worktree_id = Some(session.worktree_id.0.to_string());
    ops_event.provider_id = Some(session.provider_id.clone());
    ops_event.meta = Some(serde_json::json!({
        "model_id": session.model_id.clone(),
        "env_target": env_target.clone(),
        "parent_session_id": session.parent_session_id.map(|id| id.0.to_string()),
        "relationship": session.relationship.clone(),
    }));
    state.telemetry.ops_events.emit(ops_event);
    if let Err(e) = state.emit_workspace_task_upsert(session.task_id).await {
        tracing::warn!(task_id = %session.task_id.0, "workspace active snapshot refresh failed: {e:?}");
    }

    Ok(Json(SessionWithEnv {
        env_target,
        session,
    }))
}

pub(super) async fn load_workspace_active_snapshot_state(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
) -> (i64, i64) {
    state
        .workspaces
        .workspace_active_snapshot
        .snapshot_state(workspace_id)
        .await
}
