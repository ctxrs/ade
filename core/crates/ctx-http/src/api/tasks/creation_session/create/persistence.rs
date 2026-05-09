use super::*;

pub(super) struct PersistCreatedSession<'a> {
    pub(super) state: &'a Arc<AppState>,
    pub(super) store: &'a Store,
    pub(super) task: &'a Task,
    pub(super) workspace: &'a Workspace,
    pub(super) requested_session_id: Option<SessionId>,
    pub(super) created_worktree_id: Option<WorktreeId>,
    pub(super) worktree_id: WorktreeId,
    pub(super) execution_environment: ExecutionEnvironment,
    pub(super) provider_id: &'a str,
    pub(super) model_id: &'a str,
    pub(super) reasoning_effort: Option<&'a str>,
    pub(super) parent_session_id: Option<SessionId>,
    pub(super) relationship: Option<&'a str>,
    pub(super) requested_relationship: Option<&'a str>,
}

pub(super) async fn persist_created_session(
    request: PersistCreatedSession<'_>,
) -> Result<Session, StatusCode> {
    let PersistCreatedSession {
        state,
        store,
        task,
        workspace,
        requested_session_id,
        created_worktree_id,
        worktree_id,
        execution_environment,
        provider_id,
        model_id,
        reasoning_effort,
        parent_session_id,
        relationship,
        requested_relationship,
    } = request;

    let session = create_session_record(CreateSessionRecord {
        store,
        task_id: task.id,
        workspace_id: task.workspace_id,
        requested_session_id,
        worktree_id,
        execution_environment,
        provider_id,
        model_id,
        reasoning_effort,
        parent_session_id,
        relationship,
        cleanup: CreatedWorktreeCleanup {
            state,
            store,
            workspace,
            task_id: task.id,
            created_worktree_id,
        },
    })
    .await?;

    if let Some(session_id) = requested_session_id {
        if session.id != session_id
            || !session_matches_creation_identity(
                &session,
                SessionCreationIdentity {
                    task_id: task.id,
                    workspace_id: task.workspace_id,
                    worktree_id,
                    execution_environment,
                    provider_id,
                    model_id,
                    reasoning_effort,
                    parent_session_id,
                    relationship: requested_relationship,
                },
            )
        {
            return Err(StatusCode::CONFLICT);
        }
    }

    state.sessions.remember_session_meta(&session).await;
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

    Ok(session)
}

struct CreateSessionRecord<'a> {
    store: &'a Store,
    task_id: TaskId,
    workspace_id: WorkspaceId,
    requested_session_id: Option<SessionId>,
    worktree_id: WorktreeId,
    execution_environment: ExecutionEnvironment,
    provider_id: &'a str,
    model_id: &'a str,
    reasoning_effort: Option<&'a str>,
    parent_session_id: Option<SessionId>,
    relationship: Option<&'a str>,
    cleanup: CreatedWorktreeCleanup<'a>,
}

async fn create_session_record(request: CreateSessionRecord<'_>) -> Result<Session, StatusCode> {
    let CreateSessionRecord {
        store,
        task_id,
        workspace_id,
        requested_session_id,
        worktree_id,
        execution_environment,
        provider_id,
        model_id,
        reasoning_effort,
        parent_session_id,
        relationship,
        cleanup,
    } = request;

    let result = if let Some(session_id) = requested_session_id {
        store
            .create_session_with_id_and_reasoning_effort(
                session_id,
                task_id,
                workspace_id,
                worktree_id,
                execution_environment,
                provider_id.to_string(),
                model_id.to_string(),
                reasoning_effort.map(str::to_string),
                "implementer".to_string(),
                parent_session_id,
                relationship.map(str::to_string),
                None,
            )
            .await
    } else {
        store
            .create_session_with_reasoning_effort(
                task_id,
                workspace_id,
                worktree_id,
                execution_environment,
                provider_id.to_string(),
                model_id.to_string(),
                reasoning_effort.map(str::to_string),
                "implementer".to_string(),
                parent_session_id,
                relationship.map(str::to_string),
                None,
            )
            .await
    };

    match result {
        Ok(session) => Ok(session),
        Err(_) => {
            cleanup.cleanup_orphaned_worktree().await;
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

struct CreatedWorktreeCleanup<'a> {
    state: &'a Arc<AppState>,
    store: &'a Store,
    workspace: &'a Workspace,
    task_id: TaskId,
    created_worktree_id: Option<WorktreeId>,
}

impl CreatedWorktreeCleanup<'_> {
    async fn cleanup_orphaned_worktree(&self) {
        if let Some(created_worktree_id) = self.created_worktree_id {
            cleanup_orphaned_provisioned_worktree(
                self.state,
                self.store,
                self.workspace,
                self.task_id,
                created_worktree_id,
            )
            .await;
        }
    }
}
