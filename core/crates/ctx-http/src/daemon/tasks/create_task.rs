use std::path::Path as StdPath;

use ctx_core::ids::{TaskId, WorkspaceId};
use ctx_core::models::{ExecutionEnvironment, Task, Workspace};
use ctx_observability::logs;
use ctx_store::Store;

use crate::daemon::handle::TasksHandle;
use crate::daemon::workspaces::execution_environment_from_settings;
use crate::daemon::{DaemonHandle, ProvidersHandle, SessionsHandle, WorkspacesHandle};

#[path = "create_task/default_session_flow.rs"]
mod default_session_flow;
#[path = "create_task/default_session_plan.rs"]
mod default_session_plan;
#[path = "create_task/idempotency.rs"]
mod idempotency;
#[path = "create_task/workspace.rs"]
mod workspace;

use default_session_flow::ensure_default_session_for_task;
use default_session_plan::{preflight_default_session_creation, DefaultSessionPlan};
use idempotency::{
    load_existing_task_for_request, persist_task_for_request, reload_or_retry_task_for_request,
    upsert_workspace_task_index,
};
use workspace::load_create_task_workspace;

use super::CreateTaskSessionInput;

type CreateTaskApiError = TaskCreateError;

#[derive(Debug, Clone)]
pub(crate) struct CreateTaskInput {
    pub(crate) task_id: Option<TaskId>,
    pub(crate) title: String,
    pub(crate) description: Option<String>,
    pub(crate) default_session: Option<CreateTaskSessionInput>,
}

#[derive(Debug)]
pub(crate) enum TaskCreateError {
    BadRequest(String),
    NotFound(String),
    Conflict(String),
    Internal(anyhow::Error),
    DefaultSessionFailed(super::TaskSessionCreateError),
    DefaultSessionConflict,
}

impl TaskCreateError {
    fn internal(error: impl Into<anyhow::Error>) -> Self {
        Self::Internal(error.into())
    }
}

#[derive(Clone)]
struct TaskCreationHandles {
    tasks: TasksHandle,
    sessions: SessionsHandle,
    providers: ProvidersHandle,
    workspaces: WorkspacesHandle,
}

impl TaskCreationHandles {
    fn new(tasks: &TasksHandle) -> Self {
        let daemon = DaemonHandle::new(tasks.state.clone());
        Self {
            tasks: tasks.clone(),
            sessions: daemon.sessions(),
            providers: daemon.providers(),
            workspaces: daemon.workspaces(),
        }
    }
}

impl TasksHandle {
    pub(crate) async fn create_task_for_workspace(
        &self,
        workspace_id: WorkspaceId,
        input: CreateTaskInput,
    ) -> Result<Task, TaskCreateError> {
        let handles = TaskCreationHandles::new(self);
        let (workspace, store) = load_create_task_workspace(&handles, workspace_id).await?;
        let existing_task =
            load_existing_task_for_request(&handles, &store, workspace_id, &input).await?;
        let default_session_plan = if input.should_preflight_default_session(&existing_task) {
            Some(preflight_default_session_creation(&handles, &store, &workspace).await?)
        } else {
            None
        };
        let persisted_task =
            persist_task_for_request(&store, workspace_id, existing_task, &input).await?;
        upsert_workspace_task_index(&handles, persisted_task.task.id, workspace_id).await;

        let default_session_lock = handles
            .sessions
            .task_session_creation_lock(persisted_task.task.id)
            .await;
        let _default_session_guard = default_session_lock.lock().await;
        let persisted_task =
            reload_or_retry_task_for_request(&store, workspace_id, persisted_task, &input).await?;
        ensure_default_session_for_task(
            &handles,
            store,
            workspace,
            persisted_task.task,
            input.default_session,
            default_session_plan,
            persisted_task.created_in_this_request,
        )
        .await
    }
}

impl CreateTaskInput {
    fn should_preflight_default_session(&self, existing_task: &Option<Task>) -> bool {
        existing_task.is_none() && self.default_session.is_none()
    }
}

fn task_request_matches(existing: &Task, title: &str, description: &Option<String>) -> bool {
    existing.title == title && existing.description.as_deref() == description.as_deref()
}
