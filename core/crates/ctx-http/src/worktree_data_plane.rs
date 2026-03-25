use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Result};

use ctx_core::models::{Workspace, Worktree};
use ctx_fs::worktrees::managed_worktree_path;

use crate::container_fs::is_container_path;
use crate::daemon::AppState;
use crate::disk_isolated;
use crate::execution_effective;
use crate::harness_runtime::CTX_CONTAINER_WORKSPACE_ROOT;
use crate::settings::ExecutionMode;

#[derive(Debug, Clone)]
pub(crate) struct WorktreeDataPlane {
    pub workspace: Workspace,
    pub execution_mode: ExecutionMode,
    pub live_worktree_root: PathBuf,
}

pub(crate) fn sandbox_workspace_root() -> PathBuf {
    PathBuf::from(CTX_CONTAINER_WORKSPACE_ROOT)
}

pub(crate) fn sandbox_worktree_root(
    data_root: &Path,
    workspace: &Workspace,
    worktree: &Worktree,
) -> PathBuf {
    let root = PathBuf::from(&worktree.root_path);
    if is_container_path(&root) {
        return root;
    }
    if worktree.root_path == workspace.root_path {
        return sandbox_workspace_root();
    }
    let managed_root = managed_worktree_path(data_root, workspace.id, worktree.id);
    if Path::new(&worktree.root_path) == managed_root.as_path() {
        return disk_isolated::container_worktree_root(worktree.id);
    }
    sandbox_workspace_root()
}

pub(crate) fn live_workspace_root_for_mode(workspace: &Workspace, mode: ExecutionMode) -> PathBuf {
    match mode {
        ExecutionMode::Host => PathBuf::from(&workspace.root_path),
        ExecutionMode::Container => sandbox_workspace_root(),
    }
}

pub(crate) fn live_worktree_root_for_mode(
    data_root: &Path,
    workspace: &Workspace,
    worktree: &Worktree,
    mode: ExecutionMode,
) -> PathBuf {
    match mode {
        ExecutionMode::Host => PathBuf::from(&worktree.root_path),
        ExecutionMode::Container => sandbox_worktree_root(data_root, workspace, worktree),
    }
}

pub(crate) fn map_host_path_to_live_path(
    data_root: &Path,
    workspace: &Workspace,
    worktree: Option<&Worktree>,
    requested: &Path,
    mode: ExecutionMode,
) -> Option<PathBuf> {
    if matches!(mode, ExecutionMode::Host) {
        return Some(requested.to_path_buf());
    }
    let live_workspace_root = sandbox_workspace_root();
    if requested.starts_with(&live_workspace_root) {
        return Some(requested.to_path_buf());
    }

    if let Some(worktree) = worktree {
        let live_worktree_root = sandbox_worktree_root(data_root, workspace, worktree);
        if requested.starts_with(&live_worktree_root) {
            return Some(requested.to_path_buf());
        }
        let host_worktree_root = PathBuf::from(&worktree.root_path);
        if requested.starts_with(&host_worktree_root) {
            let relative = requested.strip_prefix(&host_worktree_root).ok()?;
            return Some(live_worktree_root.join(relative));
        }
    }

    let host_workspace_root = PathBuf::from(&workspace.root_path);
    if requested.starts_with(&host_workspace_root) {
        let relative = requested.strip_prefix(&host_workspace_root).ok()?;
        return Some(live_workspace_root.join(relative));
    }

    None
}

pub(crate) async fn resolve_worktree_data_plane(
    state: &Arc<AppState>,
    worktree: &Worktree,
) -> Result<WorktreeDataPlane> {
    let workspace = state
        .global_store()
        .get_workspace(worktree.workspace_id)
        .await?
        .ok_or_else(|| anyhow!("workspace not found for worktree"))?;
    let effective = execution_effective::effective_execution_settings(state, workspace.id).await?;
    let execution_mode = effective.mode.clone();
    let live_worktree_root = live_worktree_root_for_mode(
        &state.core.data_root,
        &workspace,
        worktree,
        execution_mode.clone(),
    );
    Ok(WorktreeDataPlane {
        execution_mode,
        live_worktree_root,
        workspace,
    })
}
