use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};

use ctx_core::models::{SandboxBinding, SandboxRuntimeFamily, Workspace, Worktree};

use crate::daemon::AppState;
use crate::disk_isolated;
use crate::harness_runtime::CTX_CONTAINER_WORKSPACE_ROOT;
use crate::settings::{ContainerMountMode, ContainerRuntimeKind, ExecutionMode, ExecutionSettings};

#[derive(Debug, Clone)]
pub(crate) struct WorktreeDataPlane {
    pub binding: Option<SandboxBinding>,
    pub workspace: Workspace,
    pub execution_mode: ExecutionMode,
    pub live_workspace_root: PathBuf,
    pub live_worktree_root: PathBuf,
}

pub(crate) fn map_host_or_live_path_to_live_roots(
    live_workspace_root: &Path,
    live_worktree_root: &Path,
    host_workspace_root: &Path,
    host_worktree_root: Option<&Path>,
    requested: &Path,
) -> Option<PathBuf> {
    if requested.starts_with(live_workspace_root) || requested.starts_with(live_worktree_root) {
        return Some(requested.to_path_buf());
    }

    if let Some(host_worktree_root) = host_worktree_root {
        if requested.starts_with(host_worktree_root) {
            let relative = requested.strip_prefix(host_worktree_root).ok()?;
            return Some(live_worktree_root.join(relative));
        }
    }

    if requested.starts_with(host_workspace_root) {
        let relative = requested.strip_prefix(host_workspace_root).ok()?;
        return Some(live_workspace_root.join(relative));
    }

    None
}

pub(crate) fn map_host_or_live_path_to_live_path(
    data_plane: &WorktreeDataPlane,
    host_workspace_root: &Path,
    host_worktree_root: Option<&Path>,
    requested: &Path,
) -> Option<PathBuf> {
    map_host_or_live_path_to_live_roots(
        &data_plane.live_workspace_root,
        &data_plane.live_worktree_root,
        host_workspace_root,
        host_worktree_root,
        requested,
    )
}

pub(crate) fn sandbox_workspace_root() -> PathBuf {
    PathBuf::from(CTX_CONTAINER_WORKSPACE_ROOT)
}

pub(crate) fn sandbox_worktree_root(workspace: &Workspace, worktree: &Worktree) -> PathBuf {
    if worktree.root_path == workspace.root_path {
        return sandbox_workspace_root();
    }
    // Sandbox worktree roots are runtime-managed and deterministic by worktree id.
    // Callers should not infer sandbox semantics from stored host-path shapes.
    disk_isolated::container_worktree_root(worktree.id)
}

pub(crate) fn live_workspace_root_for_mode(workspace: &Workspace, mode: ExecutionMode) -> PathBuf {
    match mode {
        ExecutionMode::Host => PathBuf::from(&workspace.root_path),
        ExecutionMode::Sandbox => sandbox_workspace_root(),
    }
}

pub(crate) fn live_worktree_root_for_mode(
    workspace: &Workspace,
    worktree: &Worktree,
    mode: ExecutionMode,
) -> PathBuf {
    match mode {
        ExecutionMode::Host => PathBuf::from(&worktree.root_path),
        ExecutionMode::Sandbox => sandbox_worktree_root(workspace, worktree),
    }
}

pub(crate) async fn resolve_worktree_data_plane(
    state: &AppState,
    worktree: &Worktree,
) -> Result<WorktreeDataPlane> {
    let workspace = state
        .global_store()
        .get_workspace(worktree.workspace_id)
        .await?
        .ok_or_else(|| anyhow!("workspace not found for worktree"))?;
    let store = state.store_for_workspace(worktree.workspace_id).await?;
    let binding = store.get_sandbox_binding(worktree.id).await?;
    if binding.is_none() {
        let sessions = store.list_sessions_for_worktree(worktree.id).await?;
        if sessions.iter().any(|session| {
            matches!(
                session.execution_environment,
                ctx_core::models::ExecutionEnvironment::Sandbox
            )
        }) {
            return Err(anyhow!(
                "sandbox binding is missing for sandbox worktree {}",
                worktree.id.0
            ));
        }
    }
    let execution_mode = if binding.is_some() {
        ExecutionMode::Sandbox
    } else {
        ExecutionMode::Host
    };
    let live_workspace_root = binding
        .as_ref()
        .map(|binding| PathBuf::from(&binding.live_workspace_root))
        .unwrap_or_else(|| PathBuf::from(&workspace.root_path));
    let live_worktree_root = binding
        .as_ref()
        .map(|binding| PathBuf::from(&binding.live_worktree_root))
        .unwrap_or_else(|| PathBuf::from(&worktree.root_path));
    Ok(WorktreeDataPlane {
        binding,
        execution_mode,
        live_workspace_root,
        live_worktree_root,
        workspace,
    })
}

pub(crate) fn workspace_data_plane(
    workspace: &Workspace,
    execution_mode: ExecutionMode,
) -> WorktreeDataPlane {
    let live_workspace_root = live_workspace_root_for_mode(workspace, execution_mode.clone());
    let live_worktree_root = live_workspace_root.clone();
    WorktreeDataPlane {
        binding: None,
        workspace: workspace.clone(),
        execution_mode,
        live_workspace_root,
        live_worktree_root,
    }
}

pub(crate) fn binding_runtime_kind(binding: &SandboxBinding) -> ContainerRuntimeKind {
    match binding.runtime_family {
        SandboxRuntimeFamily::NativeContainer => ContainerRuntimeKind::NativeContainer,
        SandboxRuntimeFamily::SharedVmContainer => ContainerRuntimeKind::SharedVmContainer,
    }
}

pub(crate) fn apply_data_plane_to_execution_settings(
    base: &ExecutionSettings,
    data_plane: &WorktreeDataPlane,
) -> Result<ExecutionSettings> {
    let mut settings = base.clone();
    settings.mode = data_plane.execution_mode.clone();
    if let Some(binding) = data_plane.binding.as_ref() {
        if let Some(raw) = binding.execution_settings_json.as_deref() {
            return serde_json::from_str::<ExecutionSettings>(raw).map_err(|err| {
                anyhow!(
                    "sandbox binding {} had invalid execution settings snapshot: {err:#}",
                    binding.worktree_id.0
                )
            });
        }
        settings.mode = ExecutionMode::Sandbox;
        settings.container.runtime = binding_runtime_kind(binding);
        settings.container.mount_mode = ContainerMountMode::DiskIsolated;
    } else if matches!(data_plane.execution_mode, ExecutionMode::Sandbox) {
        // Workspace-scoped sandbox flows have no binding yet, but the product contract is still
        // disk-isolated sandbox rather than a legacy host-mounted variant.
        settings.container.mount_mode = ContainerMountMode::DiskIsolated;
    }
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use ctx_core::ids::{WorkspaceId, WorktreeId};
    use uuid::Uuid;

    use super::*;
    use crate::settings::{ContainerNetworkMode, ContainerRuntimeKind};

    #[test]
    fn binding_snapshot_overrides_mutated_workspace_defaults() {
        let snapshot = ExecutionSettings {
            mode: ExecutionMode::Sandbox,
            container: crate::settings::ContainerExecutionSettings {
                runtime: ContainerRuntimeKind::SharedVmContainer,
                network_mode: ContainerNetworkMode::Allowlist,
                allowlist: vec!["github.com".to_string()],
                image: Some("registry.example/sandbox:v1".to_string()),
                ..crate::settings::ContainerExecutionSettings::default()
            },
        };
        let data_plane = WorktreeDataPlane {
            binding: Some(SandboxBinding {
                worktree_id: WorktreeId(Uuid::new_v4()),
                workspace_id: WorkspaceId(Uuid::new_v4()),
                runtime_family: SandboxRuntimeFamily::SharedVmContainer,
                profile: ctx_core::models::SandboxProfile::Standard,
                live_workspace_root: "/ctx/ws".to_string(),
                live_worktree_root: "/ctx/wt".to_string(),
                execution_settings_json: Some(
                    serde_json::to_string(&snapshot).expect("serialize snapshot"),
                ),
                container_name: Some("ctx-harness-test".to_string()),
                host_projection_root: None,
                created_at: Utc::now(),
            }),
            workspace: Workspace {
                id: WorkspaceId(Uuid::new_v4()),
                name: "ws".to_string(),
                root_path: "/host/ws".to_string(),
                created_at: Utc::now(),
                vcs_kind: None,
            },
            execution_mode: ExecutionMode::Sandbox,
            live_workspace_root: PathBuf::from("/ctx/ws"),
            live_worktree_root: PathBuf::from("/ctx/wt"),
        };

        let current = ExecutionSettings {
            mode: ExecutionMode::Sandbox,
            container: crate::settings::ContainerExecutionSettings {
                runtime: ContainerRuntimeKind::NativeContainer,
                network_mode: ContainerNetworkMode::All,
                allowlist: vec![],
                image: Some("registry.example/sandbox:v2".to_string()),
                ..crate::settings::ContainerExecutionSettings::default()
            },
        };

        let applied =
            apply_data_plane_to_execution_settings(&current, &data_plane).expect("apply settings");
        assert_eq!(
            applied.container.runtime,
            ContainerRuntimeKind::SharedVmContainer
        );
        assert_eq!(
            applied.container.network_mode,
            ContainerNetworkMode::Allowlist
        );
        assert_eq!(applied.container.allowlist, vec!["github.com".to_string()]);
        assert_eq!(
            applied.container.image,
            Some("registry.example/sandbox:v1".to_string())
        );
    }

    #[test]
    fn workspace_data_plane_uses_workspace_root_for_host_mode() {
        let workspace = Workspace {
            id: WorkspaceId(Uuid::new_v4()),
            name: "ws".to_string(),
            root_path: "/host/ws".to_string(),
            created_at: Utc::now(),
            vcs_kind: None,
        };

        let data_plane = workspace_data_plane(&workspace, ExecutionMode::Host);
        assert_eq!(data_plane.execution_mode, ExecutionMode::Host);
        assert_eq!(data_plane.live_workspace_root, PathBuf::from("/host/ws"));
        assert_eq!(data_plane.live_worktree_root, PathBuf::from("/host/ws"));
        assert!(data_plane.binding.is_none());
    }

    #[test]
    fn workspace_data_plane_uses_container_workspace_root_for_sandbox_mode() {
        let workspace = Workspace {
            id: WorkspaceId(Uuid::new_v4()),
            name: "ws".to_string(),
            root_path: "/host/ws".to_string(),
            created_at: Utc::now(),
            vcs_kind: None,
        };

        let data_plane = workspace_data_plane(&workspace, ExecutionMode::Sandbox);
        assert_eq!(data_plane.execution_mode, ExecutionMode::Sandbox);
        assert_eq!(data_plane.live_workspace_root, PathBuf::from("/ctx/ws"));
        assert_eq!(data_plane.live_worktree_root, PathBuf::from("/ctx/ws"));
        assert!(data_plane.binding.is_none());
    }

    #[test]
    fn synthetic_sandbox_data_plane_forces_disk_isolated_mount_mode() {
        let workspace = Workspace {
            id: WorkspaceId(Uuid::new_v4()),
            name: "ws".to_string(),
            root_path: "/host/ws".to_string(),
            created_at: Utc::now(),
            vcs_kind: None,
        };
        let data_plane = workspace_data_plane(&workspace, ExecutionMode::Sandbox);
        let base = ExecutionSettings {
            mode: ExecutionMode::Sandbox,
            container: crate::settings::ContainerExecutionSettings {
                mount_mode: ContainerMountMode::Legacy,
                ..crate::settings::ContainerExecutionSettings::default()
            },
        };

        let applied =
            apply_data_plane_to_execution_settings(&base, &data_plane).expect("apply settings");
        assert_eq!(applied.mode, ExecutionMode::Sandbox);
        assert_eq!(
            applied.container.mount_mode,
            ContainerMountMode::DiskIsolated
        );
    }
}
