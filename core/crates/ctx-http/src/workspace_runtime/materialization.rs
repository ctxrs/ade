use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ctx_core::models::{Workspace, Worktree};
use ctx_fs::worktrees::standaloneize_worktree_git_dir;

use crate::daemon::AppState;
use crate::settings::{ContainerMountMode, ExecutionMode, ExecutionSettings};

use super::{SharedVmLifecycleOrchestrator, UbuntuSandboxSubstrate};

#[derive(Debug, Clone)]
pub(crate) struct SandboxWorktreeMaterialization {
    pub(crate) substrate: UbuntuSandboxSubstrate,
    pub(crate) live_worktree_root: PathBuf,
    pub(crate) host_materialization_root: Option<PathBuf>,
}

pub(crate) async fn materialize_sandbox_worktree(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
    canonical_root: &Path,
    effective: &ExecutionSettings,
) -> Result<Option<SandboxWorktreeMaterialization>> {
    if !matches!(effective.mode, ExecutionMode::Sandbox)
        || !matches!(effective.container.mount_mode, ContainerMountMode::DiskIsolated)
    {
        return Ok(None);
    }

    standaloneize_worktree_git_dir(canonical_root)
        .await
        .with_context(|| {
            format!(
                "stabilizing sandbox worktree git metadata at {}",
                canonical_root.display()
            )
        })?;

    state
        .execution
        .harness
        .ensure_workspace_container(workspace, effective, &state.core.daemon_url)
        .await?;

    let substrate = UbuntuSandboxSubstrate::from_runtime_kind(effective.container.runtime);
    substrate.ensure_enabled()?;

    let branch_name = worktree
        .git_branch
        .as_deref()
        .or(worktree.vcs_ref.as_deref())
        .ok_or_else(|| anyhow::anyhow!("managed sandbox worktree is missing branch metadata"))?;

    let host_materialization_root = if substrate.is_shared_vm_backed() {
        Some(
            SharedVmLifecycleOrchestrator::new(&state.core.data_root)
                .ensure_host_materialization_root(
                    workspace.id,
                    worktree.id,
                    canonical_root,
                    &worktree.base_commit_sha,
                    branch_name,
                    None,
                )
                .await?,
        )
    } else {
        None
    };
    let host_source_root = host_materialization_root
        .as_deref()
        .unwrap_or(canonical_root);
    let live_worktree_root = crate::disk_isolated::ensure_worktree_from_host_copy(
        &state.core.data_root,
        workspace.id,
        worktree.id,
        host_source_root,
        &worktree.base_commit_sha,
        branch_name,
    )
    .await?;

    Ok(Some(SandboxWorktreeMaterialization {
        substrate,
        live_worktree_root,
        host_materialization_root,
    }))
}
