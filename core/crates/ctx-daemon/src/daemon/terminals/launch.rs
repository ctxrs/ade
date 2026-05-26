use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use ctx_core::ids::{SessionId, TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::{TerminalSession, Workspace};
use ctx_settings_model::{ExecutionMode, ExecutionSettings};
use ctx_store::Store;
use ctx_transport_runtime::terminal_launch::{container_terminal_env, TerminalLaunchError};
use ctx_transport_runtime::terminals::{TerminalCreateRequest, TerminalManager};
use ctx_workspace_runtime::HarnessRuntimeManager;
use ctx_worktree_data_plane::resolve_worktree_data_plane_with_host as resolve_worktree_data_plane;
use ctx_worktree_data_plane::{
    apply_data_plane_to_execution_settings, workspace_data_plane, WorktreeDataPlaneHost,
};

use crate::daemon::ProtectedWorkspaceStoreLookup;

mod container;
mod paths;
mod worktree;

use self::container::prepare_terminal_container_launch;
use self::paths::resolve_terminal_paths;
#[cfg(test)]
pub use self::worktree::infer_terminal_worktree;
use self::worktree::resolve_terminal_worktree;

#[derive(Debug)]
pub struct CreateTerminalLaunchRequest {
    pub workspace_id: WorkspaceId,
    pub task_id: Option<TaskId>,
    pub session_id: Option<SessionId>,
    pub worktree_id: Option<WorktreeId>,
    pub cwd: Option<String>,
    pub shell: Option<String>,
}

#[derive(Clone)]
pub(in crate::daemon) struct TerminalLaunchHost {
    global_store: Store,
    workspace_stores: ProtectedWorkspaceStoreLookup,
    data_root: PathBuf,
    daemon_url: String,
    harness: Arc<HarnessRuntimeManager>,
    terminals: Arc<TerminalManager>,
}

impl TerminalLaunchHost {
    pub(in crate::daemon) fn new(
        global_store: Store,
        workspace_stores: ProtectedWorkspaceStoreLookup,
        data_root: PathBuf,
        daemon_url: String,
        harness: Arc<HarnessRuntimeManager>,
        terminals: Arc<TerminalManager>,
    ) -> Self {
        Self {
            global_store,
            workspace_stores,
            data_root,
            daemon_url,
            harness,
            terminals,
        }
    }

    pub(super) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(super) fn daemon_url(&self) -> &str {
        &self.daemon_url
    }

    pub(super) fn harness(&self) -> &HarnessRuntimeManager {
        self.harness.as_ref()
    }

    async fn load_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Workspace, TerminalLaunchError> {
        self.global_store
            .get_workspace(workspace_id)
            .await
            .map_err(|_| internal_error("failed to load workspace"))?
            .ok_or_else(|| not_found("workspace not found"))
    }

    async fn effective_execution_settings(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<ExecutionSettings, TerminalLaunchError> {
        let store = self
            .workspace_stores
            .store_for_workspace(workspace_id)
            .await
            .map_err(|_| internal_error("failed to load execution settings"))?;
        ctx_settings_service::effective_execution_settings(&self.global_store, &store)
            .await
            .map_err(|_| internal_error("failed to load execution settings"))
    }

    async fn store_for_workspace(&self, workspace_id: WorkspaceId) -> anyhow::Result<Store> {
        self.workspace_stores
            .store_for_workspace(workspace_id)
            .await
    }

    async fn store_for_worktree(&self, worktree_id: WorktreeId) -> anyhow::Result<Store> {
        self.workspace_stores.store_for_worktree(worktree_id).await
    }

    async fn store_for_session(&self, session_id: SessionId) -> anyhow::Result<Store> {
        let workspace_id = self
            .global_store
            .get_workspace_id_for_session(session_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("workspace missing for session {}", session_id.0))?;
        self.store_for_workspace(workspace_id).await
    }

    async fn store_for_task(&self, task_id: TaskId) -> anyhow::Result<Store> {
        self.workspace_stores.store_for_task(task_id).await
    }

    async fn create_terminal(
        &self,
        request: TerminalCreateRequest,
    ) -> Result<TerminalSession, TerminalLaunchError> {
        let session = self
            .terminals
            .create(request)
            .await
            .map_err(|e| internal_error(format!("failed to create terminal: {e}")))?;
        Ok(session.snapshot())
    }
}

#[async_trait]
impl WorktreeDataPlaneHost for TerminalLaunchHost {
    async fn get_workspace(
        state: &Self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<Option<Workspace>> {
        state.global_store.get_workspace(workspace_id).await
    }

    async fn workspace_store(state: &Self, workspace_id: WorkspaceId) -> anyhow::Result<Store> {
        state.store_for_workspace(workspace_id).await
    }
}

pub(super) async fn create_workspace_terminal(
    host: &TerminalLaunchHost,
    req: CreateTerminalLaunchRequest,
) -> Result<TerminalSession, TerminalLaunchError> {
    let workspace_id = req.workspace_id;
    let workspace = host.load_workspace(workspace_id).await?;

    let effective = host.effective_execution_settings(workspace_id).await?;
    let worktree = resolve_terminal_worktree(
        host,
        workspace_id,
        req.worktree_id,
        req.session_id,
        req.task_id,
    )
    .await?;
    let worktree_data_plane = if let Some(worktree) = worktree.as_ref() {
        Some(
            resolve_worktree_data_plane(host, worktree)
                .await
                .map_err(|_| internal_error("failed to resolve worktree data plane"))?,
        )
    } else if matches!(effective.mode, ExecutionMode::Sandbox) {
        Some(workspace_data_plane(&workspace, effective.mode.clone()))
    } else {
        None
    };
    let effective = worktree_data_plane
        .as_ref()
        .map(|data_plane| {
            apply_data_plane_to_execution_settings(&effective, data_plane)
                .map_err(|_| internal_error("failed to apply worktree data plane"))
        })
        .transpose()?
        .unwrap_or(effective);
    let container_mode = matches!(effective.mode, ExecutionMode::Sandbox);
    let paths = resolve_terminal_paths(
        &workspace,
        worktree.as_ref(),
        worktree_data_plane.as_ref(),
        container_mode,
        req.cwd.as_deref(),
        req.shell.as_deref(),
    )
    .await?;

    let (cwd, native_container, shared_vm_container) = prepare_terminal_container_launch(
        host,
        &workspace,
        worktree.as_ref(),
        &effective,
        workspace_id,
        &paths.cwd,
        paths.container_cwd_authority_root.as_deref(),
    )
    .await?;
    host.create_terminal(TerminalCreateRequest {
        workspace_id,
        task_id: req.task_id,
        session_id: req.session_id,
        worktree_id: worktree.as_ref().map(|wt| wt.id),
        cwd,
        shell: paths.shell,
        cols: None,
        rows: None,
        env: if container_mode {
            container_terminal_env()
        } else {
            Default::default()
        },
        native_container,
        shared_vm_container,
    })
    .await
}

pub(super) fn not_found(error: impl Into<String>) -> TerminalLaunchError {
    TerminalLaunchError::not_found(error)
}

pub(super) fn internal_error(error: impl Into<String>) -> TerminalLaunchError {
    TerminalLaunchError::internal(error)
}

#[cfg(test)]
mod tests;
