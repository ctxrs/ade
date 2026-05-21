use ctx_core::ids::{SessionId, TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    ExecutionEnvironment, Message, Session, Task, VcsKind, Workspace, Worktree,
};
pub use ctx_session_service::session_creation::DefaultSessionSeed;
use ctx_session_service::session_creation::{
    session_matches_creation_identity, SessionCreationIdentity,
};
use ctx_session_tools::model_resolution::resolve_model_id;
use ctx_settings_model::ExecutionSettings;
use ctx_store::Store;
use std::path::Path as StdPath;

use crate::daemon::handle::TasksHandle;
use crate::daemon::scheduler::SchedulerCommand;
use crate::daemon::workspaces::{
    execution_environment_from_settings, retry_global_index_write, BranchCleanupErrorMode,
    TaskWorktreeCleanupTarget,
};
use crate::daemon::{DaemonHandle, ProvidersHandle, SessionsHandle, WorkspacesHandle};

#[path = "create_session/cleanup.rs"]
mod cleanup;
#[path = "create_session/existing.rs"]
mod existing;
#[path = "create_session/initial_prompt.rs"]
mod initial_prompt;
#[path = "create_session/loaded.rs"]
mod loaded;
#[path = "create_session/persistence.rs"]
mod persistence;
#[path = "create_session/telemetry.rs"]
mod telemetry;
#[path = "create_session/worktree.rs"]
mod worktree;

use cleanup::cleanup_orphaned_provisioned_worktree;
use existing::{resolve_existing_requested_session, ExistingRequestedSession};
use initial_prompt::{seed_initial_prompt, InitialPromptSeed};
use loaded::create_session_for_loaded_task_inner;
use telemetry::emit_session_started_observability;
use worktree::resolve_session_worktree_for_task;

#[derive(Debug, Clone)]
pub struct CreateTaskSessionInput {
    pub id: Option<String>,
    pub provider_id: String,
    pub model_id: String,
    pub reasoning_effort: Option<String>,
    pub remember_model_preference: bool,
    pub parent_session_id: Option<String>,
    pub relationship: Option<String>,
    pub initial_prompt: Option<String>,
    pub initial_message_id: Option<String>,
    pub initial_turn_id: Option<String>,
    pub worktree_id: Option<String>,
    pub execution_environment: Option<ExecutionEnvironment>,
    pub run_id_header: Option<String>,
}

impl CreateTaskSessionInput {
    pub fn from_default_seed(seed: DefaultSessionSeed) -> Self {
        Self {
            id: None,
            provider_id: seed.provider_id,
            model_id: seed.model_id,
            reasoning_effort: seed.reasoning_effort,
            remember_model_preference: false,
            parent_session_id: None,
            relationship: None,
            initial_prompt: None,
            initial_message_id: None,
            initial_turn_id: None,
            worktree_id: None,
            execution_environment: Some(seed.execution_environment),
            run_id_header: None,
        }
    }
}

#[derive(Debug)]
pub enum TaskSessionCreateError {
    BadRequest,
    NotFound,
    Conflict,
    Internal(anyhow::Error),
}

impl From<anyhow::Error> for TaskSessionCreateError {
    fn from(error: anyhow::Error) -> Self {
        Self::Internal(error)
    }
}

#[derive(Clone)]
struct TaskSessionHandles {
    sessions: SessionsHandle,
    providers: ProvidersHandle,
    workspaces: WorkspacesHandle,
}

impl TaskSessionHandles {
    fn new(handle: &TasksHandle) -> Self {
        let daemon = DaemonHandle::new(handle.state.clone());
        Self {
            sessions: daemon.sessions(),
            providers: daemon.providers(),
            workspaces: daemon.workspaces(),
        }
    }
}

impl TasksHandle {
    pub async fn create_session_for_task(
        &self,
        task_id: TaskId,
        input: CreateTaskSessionInput,
    ) -> Result<Session, TaskSessionCreateError> {
        let handles = TaskSessionHandles::new(self);
        let creation_lock = handles.sessions.task_session_creation_lock(task_id).await;
        let _creation_guard = creation_lock.lock().await;
        let context = self
            .load_task_context(task_id)
            .await
            .map_err(|error| match error {
                super::TaskLifecycleError::NotFound => TaskSessionCreateError::NotFound,
                super::TaskLifecycleError::Internal(error) => {
                    TaskSessionCreateError::Internal(error)
                }
            })?;
        let Some((store, task, workspace)) = context else {
            return Err(TaskSessionCreateError::NotFound);
        };
        create_session_for_loaded_task_inner(&handles, store, task, workspace, input).await
    }

    pub async fn create_session_for_loaded_task(
        &self,
        store: Store,
        task: Task,
        workspace: Workspace,
        input: CreateTaskSessionInput,
    ) -> Result<Session, TaskSessionCreateError> {
        let handles = TaskSessionHandles::new(self);
        create_session_for_loaded_task_inner(&handles, store, task, workspace, input).await
    }
}
