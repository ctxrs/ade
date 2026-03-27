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
mod execution;
mod handlers;
pub(in crate::api) use creation::*;
pub(crate) use execution::sandbox_execution_settings_from_binding;
pub(in crate::api) use execution::*;
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
    Session, SessionEventType, SessionTurn, SessionTurnStatus, Task, TaskDeltaKind, VcsKind,
    Workspace, WorkspaceArchivedPage, WorkspaceIndexCursor, Worktree,
};
use ctx_fs::git::delete_branch;
use ctx_fs::vcs;
use ctx_fs::worktrees::{create_worktree, managed_worktree_path};
use ctx_store::{is_unique_constraint_violation, Store};

const GLOBAL_INDEX_WRITE_RETRY_LIMIT: usize = 3;
const GLOBAL_INDEX_WRITE_RETRY_BASE_MS: u64 = 40;

fn execution_environment_from_settings(settings: &ExecutionSettings) -> ExecutionEnvironment {
    match settings.mode {
        ExecutionMode::Host => ExecutionEnvironment::Host,
        ExecutionMode::Sandbox => ExecutionEnvironment::Sandbox,
    }
}

pub(in crate::api) async fn materialize_sandbox_binding_for_worktree(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
    canonical_root: &StdPath,
    effective: &ExecutionSettings,
    created_at: DateTime<Utc>,
) -> anyhow::Result<Option<SandboxBinding>> {
    let Some(materialization) = crate::workspace_runtime::materialize_sandbox_worktree(
        state,
        workspace,
        worktree,
        canonical_root,
        effective,
    )
    .await?
    else {
        return Ok(None);
    };

    Ok(Some(SandboxBinding {
        worktree_id: worktree.id,
        workspace_id: workspace.id,
        sandbox_instance_id: materialization.sandbox_instance_id,
        substrate: materialization.substrate.substrate,
        guest_identity: materialization.substrate.guest_identity,
        profile: SandboxProfile::Standard,
        live_workspace_root: crate::harness_runtime::CTX_CONTAINER_WORKSPACE_ROOT.to_string(),
        live_worktree_root: materialization
            .live_worktree_root
            .to_string_lossy()
            .to_string(),
        execution_settings_json: Some(serde_json::to_string(effective)?),
        container_name: Some(crate::harness_runtime::workspace_container_name(
            workspace.id,
        )),
        host_materialization_root: materialization
            .host_materialization_root
            .map(|path| path.to_string_lossy().to_string()),
        created_at,
    }))
}

pub(in crate::api) async fn rematerialize_sandbox_binding_for_worktree(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
    existing_binding: &SandboxBinding,
) -> anyhow::Result<SandboxBinding> {
    let canonical_root = managed_worktree_root(state, workspace, worktree)
        .ok_or_else(|| anyhow::anyhow!("worktree is not a managed ctx worktree"))?;
    let effective = sandbox_execution_settings_from_binding(existing_binding)?;
    materialize_sandbox_binding_for_worktree(
        state,
        workspace,
        worktree,
        &canonical_root,
        &effective,
        existing_binding.created_at,
    )
    .await?
    .ok_or_else(|| anyhow::anyhow!("sandbox binding rematerialization produced host mode"))
}

#[derive(Debug, Clone)]
pub(in crate::api) struct TaskWorktreeCleanupTarget {
    pub worktree: Worktree,
    pub sandbox_binding: Option<SandboxBinding>,
    pub managed_root: Option<PathBuf>,
    pub destroy_worktree_on_cleanup: bool,
}

pub(in crate::api) async fn persist_provisioned_worktree(
    state: &Arc<AppState>,
    store: &Store,
    workspace: &Workspace,
    worktree: Worktree,
    sandbox_binding: Option<SandboxBinding>,
) -> anyhow::Result<Worktree> {
    store.insert_worktree(worktree.clone()).await?;
    if let Some(binding) = sandbox_binding {
        store.upsert_sandbox_binding(binding).await?;
    }
    retry_global_index_write(|| async {
        state
            .global_store()
            .upsert_workspace_worktree_index(worktree.id, workspace.id)
            .await
    })
    .await?;

    if let Err(err) = worktree_bootstrap::spawn_worktree_bootstrap(
        Arc::clone(state),
        workspace.clone(),
        worktree.clone(),
    )
    .await
    {
        tracing::warn!(worktree_id = %worktree.id.0, "worktree bootstrap failed: {err:?}");
    }
    if let Err(err) =
        attachments::sync_workspace_attachments(Arc::clone(state), workspace, false).await
    {
        tracing::warn!(worktree_id = %worktree.id.0, "attachment sync failed: {err:?}");
    }
    if let Err(err) =
        attachments::ensure_worktree_attachment_mounts_if_materialized(state, workspace, &worktree)
            .await
    {
        tracing::warn!(worktree_id = %worktree.id.0, "attachment mounts failed: {err:?}");
    }

    Ok(worktree)
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

    let worktree = Worktree {
        id: worktree_id,
        workspace_id: workspace.id,
        root_path: canonical_root.to_string_lossy().to_string(),
        base_commit_sha: base_commit_sha.to_string(),
        git_branch: Some(branch_name.to_string()),
        vcs_kind: Some(VcsKind::Git),
        base_revision: Some(base_commit_sha.to_string()),
        vcs_ref: Some(branch_name.to_string()),
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
    let binding = materialize_sandbox_binding_for_worktree(
        state,
        workspace,
        &worktree,
        &canonical_root,
        effective,
        Utc::now(),
    )
    .await?;

    Ok((canonical_root, binding))
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
    let workspace = state
        .global_store()
        .get_workspace(task.workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    let sessions = store
        .list_sessions_for_task(task_id)
        .await
        .unwrap_or_default();
    let mut worktree_ids: HashSet<WorktreeId> = sessions.iter().map(|s| s.worktree_id).collect();
    if let Some(primary_worktree_id) = task.primary_worktree_id {
        worktree_ids.insert(primary_worktree_id);
    }
    let mut cleanup_targets = Vec::new();
    for worktree_id in &worktree_ids {
        let other_active = match store
            .count_active_tasks_for_worktree(*worktree_id, Some(task_id))
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
        let other_tasks = match store
            .count_tasks_for_worktree(*worktree_id, Some(task_id))
            .await
        {
            Ok(count) => count > 0,
            Err(err) => {
                tracing::warn!(
                    task_id = %task_id.0,
                    worktree_id = %worktree_id.0,
                    "failed to check total worktree usage: {err:#}"
                );
                true
            }
        };
        let worktree = match store.get_worktree(*worktree_id).await {
            Ok(Some(worktree)) => worktree,
            Ok(None) => continue,
            Err(err) => {
                tracing::warn!(
                    task_id = %task_id.0,
                    worktree_id = %worktree_id.0,
                    "failed to load worktree for delete cleanup: {err:#}"
                );
                continue;
            }
        };
        let sandbox_binding = match store.get_sandbox_binding(*worktree_id).await {
            Ok(binding) => binding,
            Err(err) => {
                tracing::warn!(
                    task_id = %task_id.0,
                    worktree_id = %worktree_id.0,
                    "failed to load sandbox binding for delete cleanup: {err:#}"
                );
                None
            }
        };
        cleanup_targets.push(TaskWorktreeCleanupTarget {
            managed_root: managed_worktree_root(&state, &workspace, &worktree),
            sandbox_binding,
            worktree,
            destroy_worktree_on_cleanup: !other_tasks,
        });
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
    let cleanup_errors =
        cleanup_task_worktrees(state.as_ref(), &workspace, task_id, &cleanup_targets).await;
    if !cleanup_errors.is_empty() {
        tracing::warn!(
            task_id = %task_id.0,
            cleanup_errors = cleanup_errors.len(),
            "delete cleanup had errors after task row removal"
        );
    }
    let cleanup_succeeded = cleanup_errors.is_empty();
    for target in &cleanup_targets {
        if !target.destroy_worktree_on_cleanup || !cleanup_succeeded {
            continue;
        }
        let deleted_worktree_row = match store.delete_worktree(target.worktree.id).await {
            Ok(deleted) => deleted,
            Err(err) => {
                tracing::warn!(
                    task_id = %task_id.0,
                    worktree_id = %target.worktree.id.0,
                    "failed to delete worktree row after task delete: {err:#}"
                );
                false
            }
        };
        if !deleted_worktree_row {
            tracing::warn!(
                task_id = %task_id.0,
                worktree_id = %target.worktree.id.0,
                "skipping worktree index deletion because worktree row was not deleted"
            );
            continue;
        }
        if let Err(err) = state
            .global_store()
            .delete_workspace_worktree_index(target.worktree.id)
            .await
        {
            tracing::warn!(
                task_id = %task_id.0,
                worktree_id = %target.worktree.id.0,
                "failed to delete worktree index after task delete: {err:#}"
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

pub(in crate::api) async fn cleanup_task_worktrees(
    state: &AppState,
    workspace: &Workspace,
    task_id: TaskId,
    targets: &[TaskWorktreeCleanupTarget],
) -> Vec<anyhow::Error> {
    let mut errors = Vec::new();
    let mut needs_prune = false;
    for target in targets {
        let worktree = &target.worktree;
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
        if let Some(binding) = target.sandbox_binding.as_ref() {
            if let Err(err) = crate::disk_isolated::remove_live_worktree_root(
                &state.core.data_root,
                workspace.id,
                StdPath::new(&binding.live_worktree_root),
            )
            .await
            {
                tracing::warn!(
                    task_id = %task_id.0,
                    worktree_id = %worktree.id.0,
                    live_worktree_root = binding.live_worktree_root,
                    "failed to remove sandbox live worktree root: {err:#}"
                );
                errors.push(err);
            }
            if let Some(host_materialization_root) = binding.host_materialization_root.as_deref() {
                let host_materialization_root = PathBuf::from(host_materialization_root);
                if tokio::fs::metadata(&host_materialization_root).await.is_ok() {
                    if let Err(err) = tokio::fs::remove_dir_all(&host_materialization_root)
                        .await
                        .with_context(|| {
                            format!(
                                "removing sandbox host materialization root at {}",
                                host_materialization_root.display()
                            )
                        })
                    {
                        tracing::warn!(
                            task_id = %task_id.0,
                            worktree_id = %worktree.id.0,
                            host_materialization_root = %host_materialization_root.display(),
                            "failed to remove sandbox host materialization root: {err:#}"
                        );
                        errors.push(err);
                    }
                }
            }
        }
        if !target.destroy_worktree_on_cleanup {
            continue;
        }
        let Some(root) = target.managed_root.as_ref() else {
            continue;
        };
        let branch = worktree
            .git_branch
            .as_deref()
            .filter(|name| name.starts_with("ctx/"));
        if tokio::fs::metadata(root).await.is_err() {
            if branch.is_some() {
                needs_prune = true;
            }
            if target.destroy_worktree_on_cleanup {
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
            continue;
        }
        let embedded_git_dir = tokio::fs::metadata(root.join(".git"))
            .await
            .map(|meta| meta.is_dir())
            .unwrap_or(false);
        let is_git = embedded_git_dir || is_git_worktree(root).await.unwrap_or(false);
        if embedded_git_dir {
            if let Err(err) = tokio::fs::remove_dir_all(root).await.with_context(|| {
                format!("removing standalone managed worktree at {}", root.display())
            }) {
                tracing::warn!(
                    task_id = %task_id.0,
                    worktree_id = %worktree.id.0,
                    "failed to remove standalone managed worktree dir: {err:#}"
                );
                errors.push(err);
            }
        } else if is_git {
            needs_prune = true;
            if let Err(err) = remove_worktree(&workspace.root_path, root).await {
                tracing::warn!(
                    task_id = %task_id.0,
                    worktree_id = %worktree.id.0,
                    "failed to remove worktree: {err:#}"
                );
                errors.push(err);
                continue;
            }
            if tokio::fs::metadata(root).await.is_ok() {
                if let Err(err) = tokio::fs::remove_dir_all(root)
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
        } else if let Err(err) = tokio::fs::remove_dir_all(root)
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
        if target.destroy_worktree_on_cleanup {
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
    }
    if needs_prune {
        if let Err(err) = prune_worktrees(&workspace.root_path).await {
            tracing::warn!(task_id = %task_id.0, "failed to prune worktrees: {err:#}");
            errors.push(err);
        }
    }
    errors
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

pub(crate) async fn ensure_worktree_attached(
    workspace_root: impl AsRef<StdPath>,
    worktree_path: impl AsRef<StdPath>,
    base_commit_sha: &str,
    branch_name: &str,
) -> anyhow::Result<()> {
    let workspace_root = workspace_root.as_ref();
    let worktree_path = worktree_path.as_ref();
    if let Some(parent) = worktree_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .context("creating worktree parent dir")?;
    }

    let mut prune_stale_registration = false;
    match tokio::fs::metadata(worktree_path).await {
        Ok(metadata) => {
            if is_git_worktree(worktree_path).await.unwrap_or(false) {
                return Ok(());
            }
            if metadata.is_dir() {
                tokio::fs::remove_dir_all(worktree_path)
                    .await
                    .context("removing stale worktree dir")?;
            } else {
                tokio::fs::remove_file(worktree_path)
                    .await
                    .context("removing stale worktree file")?;
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            // Missing managed roots can still remain registered in `git worktree list`.
            // Prune those stale registrations before reattaching the canonical path.
            prune_stale_registration = true;
        }
        Err(err) => {
            return Err(err).context("reading managed worktree root metadata");
        }
    }

    if prune_stale_registration {
        prune_worktrees(workspace_root)
            .await
            .context("pruning stale managed worktree registrations")?;
    }

    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(workspace_root)
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
mod lifecycle_tests;

#[cfg(test)]
mod tests;
