use super::*;
use crate::api::shared::store_for_existing_workspace_status;

#[derive(Debug, Serialize)]
pub(in crate::api) struct ArchiveTaskResponse {
    #[serde(flatten)]
    task: Task,
    cleanup_failed: bool,
}

pub(in crate::api) async fn archive_task(
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
    let session_ids: Vec<SessionId> = store
        .list_sessions_for_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .into_iter()
        .map(|session| session.id)
        .collect();
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

    let updated = store
        .archive_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !updated {
        return Err(StatusCode::NOT_FOUND);
    }
    let mut errors: Vec<anyhow::Error> = Vec::new();
    let mut cleanup_targets = Vec::new();
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
        let sandbox_binding = match store.get_sandbox_binding(worktree.id).await {
            Ok(binding) => binding,
            Err(err) => {
                tracing::warn!(
                    task_id = %task_id.0,
                    worktree_id = %worktree.id.0,
                    "failed to load sandbox binding for cleanup: {err:#}"
                );
                None
            }
        };
        cleanup_targets.push(TaskWorktreeCleanupTarget {
            managed_root: managed_worktree_root(&state, &workspace, worktree),
            sandbox_binding,
            worktree: worktree.clone(),
            destroy_worktree_on_cleanup: false,
        });
    }
    errors.extend(
        cleanup_task_worktrees(state.as_ref(), &workspace, task_id, &cleanup_targets).await,
    );
    let cleanup_failed = !errors.is_empty();
    if cleanup_failed {
        tracing::warn!(
            task_id = %task_id.0,
            "archive cleanup had errors after task state was persisted"
        );
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
    for session_id in session_ids {
        state
            .workspaces
            .workspace_active_snapshot
            .remove_session(session_id)
            .await;
    }
    Ok(Json(ArchiveTaskResponse {
        task,
        cleanup_failed,
    }))
}

pub(in crate::api) async fn unarchive_task(
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
    let session_ids: Vec<SessionId> = sessions.iter().map(|session| session.id).collect();
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
        let sandbox_binding = store
            .get_sandbox_binding(worktree.id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if let Some(binding) = sandbox_binding.as_ref() {
            let refreshed_binding =
                rematerialize_sandbox_binding_for_worktree(&state, &workspace, worktree, binding)
                    .await
                    .map_err(|err| {
                        tracing::warn!(
                            task_id = %task_id.0,
                            worktree_id = %worktree.id.0,
                            "failed to rematerialize sandbox worktree on unarchive: {err:#}"
                        );
                        StatusCode::INTERNAL_SERVER_ERROR
                    })?;
            store
                .upsert_sandbox_binding(refreshed_binding)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
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
        if let Err(e) =
            vcs_hooks::ensure_task_commit_hook(&state, &workspace, worktree, task_id).await
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
    for session_id in session_ids {
        state
            .workspaces
            .workspace_active_snapshot
            .remove_session(session_id)
            .await;
        state.refresh_session_head_cache(session_id).await;
    }
    Ok(Json(task))
}

pub(in crate::api) async fn mark_task_read(
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

pub(in crate::api) async fn mark_task_unread(
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

pub(in crate::api) async fn list_workspace_tasks(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<Task>>, StatusCode> {
    let workspace_id =
        WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = store_for_existing_workspace_status(&state, workspace_id).await?;
    let tasks = store
        .list_tasks(workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(tasks))
}

#[derive(Debug, Deserialize)]
pub(in crate::api) struct WorkspaceArchivedQuery {
    limit: Option<u32>,
    cursor_sort_at: Option<String>,
    cursor_task_id: Option<String>,
}

pub(in crate::api) async fn list_workspace_archived_task_summaries(
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

    let store = store_for_existing_workspace_status(&state, workspace_id).await?;
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

pub(in crate::api) async fn list_task_sessions(
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
