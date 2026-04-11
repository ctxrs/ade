use anyhow::Result;
use ctx_core::ids::SandboxInstanceId;
use ctx_core::models::{sandbox_instance_id_for_workspace, Workspace, Worktree};
use ctx_execution_runtime::{ExecutionMode, ExecutionSettings};
use ctx_sandbox_contract::ContainerMountMode;
use ctx_sandbox_materialization::ensure_worktree_from_host_copy;
use std::path::{Path, PathBuf};

use super::{HarnessRuntimeManager, SharedVmLifecycleOrchestrator, UbuntuSandboxSubstrate};

#[derive(Debug, Clone)]
pub struct SandboxWorktreeMaterialization {
    pub sandbox_instance_id: SandboxInstanceId,
    pub substrate: UbuntuSandboxSubstrate,
    pub live_worktree_root: PathBuf,
    pub host_materialization_root: Option<PathBuf>,
}

pub async fn materialize_sandbox_worktree(
    data_root: &Path,
    daemon_url: &str,
    harness: &HarnessRuntimeManager,
    workspace: &Workspace,
    worktree: &Worktree,
    canonical_root: &Path,
    effective: &ExecutionSettings,
) -> Result<Option<SandboxWorktreeMaterialization>> {
    if !matches!(effective.mode, ExecutionMode::Sandbox)
        || !matches!(
            effective.container.mount_mode,
            ContainerMountMode::DiskIsolated
        )
    {
        return Ok(None);
    }

    harness
        .ensure_workspace_container_after_machine_ready_with_observer(
            workspace, effective, daemon_url, None,
        )
        .await?;

    let substrate = UbuntuSandboxSubstrate::from_runtime_kind(effective.container.runtime.clone());
    substrate.ensure_enabled()?;

    let branch_name = worktree
        .git_branch
        .as_deref()
        .or(worktree.vcs_ref.as_deref())
        .ok_or_else(|| anyhow::anyhow!("managed sandbox worktree is missing branch metadata"))?;

    let host_materialization_root = if substrate.is_shared_vm_backed() {
        Some(
            SharedVmLifecycleOrchestrator::new(data_root)
                .ensure_host_materialization_root(
                    sandbox_instance_id_for_workspace(workspace.id),
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
    let sandbox_mode = super::selected_sandbox_command_mode(data_root)?;
    let live_worktree_root = ensure_worktree_from_host_copy(
        data_root,
        &sandbox_mode,
        workspace.id,
        worktree.id,
        host_source_root,
        &worktree.base_commit_sha,
        branch_name,
    )
    .await?;

    Ok(Some(SandboxWorktreeMaterialization {
        sandbox_instance_id: sandbox_instance_id_for_workspace(workspace.id),
        substrate,
        live_worktree_root,
        host_materialization_root,
    }))
}
