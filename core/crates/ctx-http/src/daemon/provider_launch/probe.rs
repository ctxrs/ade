use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use ctx_core::ids::WorktreeId;
use ctx_core::models::{Workspace, Worktree};
use ctx_worktree_data_plane::{
    apply_data_plane_to_execution_settings, workspace_data_plane, WorktreeDataPlane,
};

use crate::daemon::execution_effective;
use crate::daemon::AppState;
use ctx_observability::logs;
use ctx_settings_model::{ContainerMountMode, ExecutionMode};
use ctx_worktree_data_plane::resolve_worktree_data_plane_with_host as resolve_worktree_data_plane;

#[allow(unused_imports)]
pub(crate) use ctx_provider_runtime::provider_launch::probe::{
    provider_auth_context_for_workspace_runtime, provider_auth_context_for_worktree_runtime,
    provider_has_active_auth_for_workspace_runtime, provider_probe_context_for_workspace_runtime,
    provider_probe_env, provider_probe_env_for_workspace_runtime, PreparedWorkspaceProbeRuntime,
    WorkspaceRuntimeProbeContext,
};

#[async_trait]
impl ctx_provider_runtime::provider_launch::probe::ProviderProbeHost for AppState {
    fn data_root(&self) -> &Path {
        &self.core.data_root
    }

    fn daemon_url(&self) -> &str {
        &self.core.daemon_url
    }

    fn auth_token(&self) -> Option<&String> {
        self.core.auth_token.as_ref()
    }

    fn redact_sensitive(&self, input: &str) -> String {
        logs::redact_sensitive(input)
    }

    async fn load_workspace(
        &self,
        workspace_id: ctx_core::ids::WorkspaceId,
    ) -> Result<Option<Workspace>, String> {
        self.global_store()
            .get_workspace(workspace_id)
            .await
            .map_err(|err| logs::redact_sensitive(&format!("loading workspace failed: {err:#}")))
    }

    async fn prepare_workspace_probe_runtime(
        &self,
        workspace: &Workspace,
    ) -> Result<PreparedWorkspaceProbeRuntime, String> {
        let effective = execution_effective::effective_execution_settings(self, workspace.id)
            .await
            .map_err(|err| {
                logs::redact_sensitive(&format!("effective execution settings failed: {err}"))
            })?;
        if matches!(effective.mode, ExecutionMode::Host) {
            return Ok(PreparedWorkspaceProbeRuntime {
                cwd: PathBuf::from(&workspace.root_path),
                runtime_data_root: None,
                env_overrides: HashMap::new(),
            });
        }

        let worktree = synthetic_probe_worktree(workspace);
        let worktree_data_plane = workspace_data_plane(workspace, effective.mode.clone());
        let effective = apply_data_plane_to_execution_settings(&effective, &worktree_data_plane)
            .map_err(|err| {
                logs::redact_sensitive(&format!(
                    "applying workspace probe data plane failed: {err:#}"
                ))
            })?;
        let cwd = probe_cwd_for_workspace_runtime(
            &worktree_data_plane,
            &worktree,
            effective.mode.clone(),
            effective.container.mount_mode.clone(),
        );
        let runtime_plan = self
            .execution
            .harness
            .prepare(workspace, &worktree, &effective, &self.core.daemon_url)
            .await
            .map_err(|err| {
                logs::redact_sensitive(&format!("probe runtime preparation failed: {err:#}"))
            })?;
        let sandbox_mode = ctx_harness_runtime::selected_sandbox_command_mode(&self.core.data_root)
            .map_err(|err| {
                logs::redact_sensitive(&format!("sandbox command selection failed: {err:#}"))
            })?;
        ctx_sandbox_materialization::ensure_workspace_root_from_host_copy(
            &self.core.data_root,
            &sandbox_mode,
            workspace,
        )
        .await
        .map_err(|err| {
            logs::redact_sensitive(&format!(
                "sandbox workspace root materialization failed: {err:#}"
            ))
        })?;
        let runtime_data_root = runtime_plan
            .env_overrides
            .get("CTX_DATA_ROOT")
            .map(|value| Path::new(value).to_path_buf());
        Ok(PreparedWorkspaceProbeRuntime {
            cwd,
            runtime_data_root,
            env_overrides: runtime_plan.env_overrides,
        })
    }

    async fn prepare_worktree_probe_runtime(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
    ) -> Result<PreparedWorkspaceProbeRuntime, String> {
        let effective = execution_effective::effective_execution_settings(self, workspace.id)
            .await
            .map_err(|err| {
                logs::redact_sensitive(&format!("effective execution settings failed: {err}"))
            })?;
        if matches!(effective.mode, ExecutionMode::Host) {
            return Ok(PreparedWorkspaceProbeRuntime {
                cwd: PathBuf::from(&worktree.root_path),
                runtime_data_root: None,
                env_overrides: HashMap::new(),
            });
        }

        let worktree_data_plane =
            resolve_worktree_data_plane(self, worktree)
                .await
                .map_err(|err| {
                    logs::redact_sensitive(&format!(
                        "resolving session auth worktree data plane failed: {err:#}"
                    ))
                })?;
        let effective = apply_data_plane_to_execution_settings(&effective, &worktree_data_plane)
            .map_err(|err| {
                logs::redact_sensitive(&format!(
                    "applying session auth worktree data plane failed: {err:#}"
                ))
            })?;
        let cwd = probe_cwd_for_workspace_runtime(
            &worktree_data_plane,
            worktree,
            effective.mode.clone(),
            effective.container.mount_mode.clone(),
        );
        let runtime_plan = self
            .execution
            .harness
            .prepare(workspace, worktree, &effective, &self.core.daemon_url)
            .await
            .map_err(|err| {
                logs::redact_sensitive(&format!("session auth runtime preparation failed: {err:#}"))
            })?;
        let runtime_data_root = runtime_plan
            .env_overrides
            .get("CTX_DATA_ROOT")
            .map(|value| Path::new(value).to_path_buf());
        Ok(PreparedWorkspaceProbeRuntime {
            cwd,
            runtime_data_root,
            env_overrides: runtime_plan.env_overrides,
        })
    }
}

fn synthetic_probe_worktree(workspace: &Workspace) -> Worktree {
    Worktree {
        id: WorktreeId(uuid::Uuid::nil()),
        workspace_id: workspace.id,
        root_path: workspace.root_path.clone(),
        base_commit_sha: String::new(),
        git_branch: None,
        vcs_kind: workspace.vcs_kind.clone(),
        base_revision: None,
        vcs_ref: None,
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
    }
}

fn probe_cwd_for_workspace_runtime(
    data_plane: &WorktreeDataPlane,
    worktree: &Worktree,
    mode: ExecutionMode,
    mount_mode: ContainerMountMode,
) -> PathBuf {
    if matches!(mode, ExecutionMode::Sandbox)
        && matches!(mount_mode, ContainerMountMode::DiskIsolated)
    {
        return data_plane.live_worktree_root.clone();
    }
    PathBuf::from(&worktree.root_path)
}
