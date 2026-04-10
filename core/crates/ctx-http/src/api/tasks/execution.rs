use super::*;

pub(crate) use ctx_sandbox_contract::sandbox_execution_settings_from_binding;
#[cfg(test)]
use ctx_sandbox_contract::SANDBOX_BINDING_EXECUTION_SETTINGS_SCHEMA_V1;
use ctx_worktree_data_plane::apply_data_plane_to_execution_settings;

pub(in crate::api) struct ResolvedExistingWorktreeExecution {
    pub worktree: Worktree,
    pub effective: ExecutionSettings,
}

impl ResolvedExistingWorktreeExecution {
    pub fn execution_environment(&self) -> ExecutionEnvironment {
        execution_environment_from_settings(&self.effective)
    }
}

pub(in crate::api) async fn resolve_existing_worktree_execution(
    state: &Arc<AppState>,
    store: &Store,
    workspace: &Workspace,
    worktree_id: WorktreeId,
) -> anyhow::Result<ResolvedExistingWorktreeExecution> {
    let worktree = store
        .get_worktree(worktree_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("worktree not found"))?;
    let base_effective =
        crate::execution_effective::effective_execution_settings(state, workspace.id)
            .await
            .context("loading workspace execution settings")?;
    let data_plane = crate::worktree_data_plane::resolve_worktree_data_plane(state, &worktree)
        .await
        .context("resolving worktree data plane")?;
    let effective = apply_data_plane_to_execution_settings(&base_effective, &data_plane)
        .context("applying worktree data plane to execution settings")?;
    Ok(ResolvedExistingWorktreeExecution {
        worktree,
        effective,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::AppState;
    use crate::settings::{ContainerMountMode, ContainerNetworkMode, ContainerRuntimeKind};
    use ctx_workspace_config::{self, ExecutionConfigUpdate};
    use chrono::Utc;
    use ctx_core::ids::{SandboxInstanceId, WorkspaceId, WorktreeId};
    use ctx_core::models::{
        ExecutionEnvironment, SandboxGuestIdentity, SandboxProfile, SandboxSubstrate, VcsKind,
    };
    use ctx_store::StoreManager;
    use std::collections::HashMap;
    use std::sync::Arc;
    use uuid::Uuid;

    fn test_binding(substrate: SandboxSubstrate, raw: Option<String>) -> SandboxBinding {
        let workspace_id = WorkspaceId(Uuid::new_v4());
        SandboxBinding {
            worktree_id: WorktreeId(Uuid::new_v4()),
            workspace_id,
            sandbox_instance_id: ctx_core::models::sandbox_instance_id_for_workspace(workspace_id),
            substrate,
            guest_identity: SandboxGuestIdentity::linux_container_ubuntu(),
            profile: SandboxProfile::Standard,
            live_workspace_root: "/ctx/ws".to_string(),
            live_worktree_root: "/ctx/wt".to_string(),
            execution_settings_json: raw,
            container_name: Some("ctx-test".to_string()),
            host_materialization_root: Some("/tmp/shadow".to_string()),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn binding_snapshot_plain_json_is_normalized() {
        let raw = serde_json::json!({
            "mode": "sandbox",
            "container": {
                "runtime": "shared_vm_container",
                "mount_mode": "legacy",
                "network_mode": "allowlist",
                "allowlist": ["github.com"],
                "image": "registry.example/sandbox:v1"
            }
        });
        let binding = test_binding(SandboxSubstrate::SharedVmContainer, Some(raw.to_string()));

        let parsed = sandbox_execution_settings_from_binding(&binding).expect("parse binding");

        assert_eq!(parsed.mode, ExecutionMode::Sandbox);
        assert_eq!(
            parsed.container.runtime,
            ContainerRuntimeKind::SharedVmContainer
        );
        assert_eq!(
            parsed.container.mount_mode,
            ContainerMountMode::DiskIsolated
        );
        assert_eq!(
            parsed.container.network_mode,
            ContainerNetworkMode::Allowlist
        );
        assert_eq!(parsed.container.allowlist, vec!["github.com".to_string()]);
        assert_eq!(
            parsed.container.image,
            Some("registry.example/sandbox:v1".to_string())
        );
    }

    #[test]
    fn binding_snapshot_versioned_payload_is_supported() {
        let raw = serde_json::json!({
            "schema_version": SANDBOX_BINDING_EXECUTION_SETTINGS_SCHEMA_V1,
            "execution_settings": {
                "mode": "sandbox",
                "container": {
                    "runtime": "native_container",
                    "mount_mode": "disk_isolated",
                    "network_mode": "llm_only",
                    "allowlist": [],
                    "image": null
                }
            }
        });
        let binding = test_binding(SandboxSubstrate::NativeContainer, Some(raw.to_string()));

        let parsed = sandbox_execution_settings_from_binding(&binding).expect("parse binding");

        assert_eq!(parsed.mode, ExecutionMode::Sandbox);
        assert_eq!(
            parsed.container.runtime,
            ContainerRuntimeKind::NativeContainer
        );
        assert_eq!(
            parsed.container.mount_mode,
            ContainerMountMode::DiskIsolated
        );
    }

    #[test]
    fn binding_snapshot_rejects_unknown_schema_version() {
        let raw = serde_json::json!({
            "schema_version": 99,
            "execution_settings": {
                "mode": "sandbox",
                "container": {
                    "runtime": "shared_vm_container",
                    "mount_mode": "disk_isolated"
                }
            }
        });
        let binding = test_binding(SandboxSubstrate::SharedVmContainer, Some(raw.to_string()));

        let err = sandbox_execution_settings_from_binding(&binding)
            .expect_err("unknown schema version should fail");

        assert!(format!("{err:#}")
            .contains("unsupported sandbox binding execution settings schema version 99"));
    }

    #[test]
    fn binding_snapshot_rejects_host_mode() {
        let raw = serde_json::json!({
            "mode": "host",
            "container": {
                "runtime": "native_container",
                "mount_mode": "disk_isolated",
                "network_mode": "all",
                "allowlist": [],
                "image": null
            }
        });
        let binding = test_binding(SandboxSubstrate::NativeContainer, Some(raw.to_string()));

        let err = sandbox_execution_settings_from_binding(&binding)
            .expect_err("host-mode binding snapshot should fail closed");

        assert!(format!("{err:#}")
            .contains("sandbox binding execution settings snapshot must keep mode=sandbox"));
    }

    #[test]
    fn binding_snapshot_rejects_substrate_mismatch() {
        let raw = serde_json::json!({
            "mode": "sandbox",
            "container": {
                "runtime": "shared_vm_container",
                "mount_mode": "disk_isolated",
                "network_mode": "all",
                "allowlist": [],
                "image": null
            }
        });
        let binding = test_binding(SandboxSubstrate::NativeContainer, Some(raw.to_string()));

        let err = sandbox_execution_settings_from_binding(&binding)
            .expect_err("runtime-family mismatch should fail closed");

        assert!(format!("{err:#}")
            .contains("sandbox binding execution settings snapshot runtime shared_vm_container does not match binding substrate native_container"));
    }

    #[test]
    fn binding_snapshot_rejects_non_workspace_mapped_sandbox_instance() {
        let raw = serde_json::json!({
            "mode": "sandbox",
            "container": {
                "runtime": "native_container",
                "mount_mode": "disk_isolated",
                "network_mode": "all",
                "allowlist": [],
                "image": null
            }
        });
        let mut binding = test_binding(SandboxSubstrate::NativeContainer, Some(raw.to_string()));
        binding.sandbox_instance_id = SandboxInstanceId(Uuid::new_v4());

        let err = sandbox_execution_settings_from_binding(&binding)
            .expect_err("non-workspace-mapped sandbox instance should fail closed");

        assert!(format!("{err:#}").contains("unsupported sandbox_instance_id"));
    }

    #[tokio::test]
    async fn resolve_existing_worktree_execution_uses_binding_snapshot_after_workspace_defaults_change(
    ) {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("create repo root");
        let state = Arc::new(AppState::new(
            temp.path().to_path_buf(),
            StoreManager::open(temp.path()).await.expect("open stores"),
            HashMap::new(),
            "http://127.0.0.1:4310".to_string(),
            None,
        ));
        let workspace = state
            .global_store()
            .create_workspace(
                "ws".to_string(),
                repo_root.to_string_lossy().to_string(),
                VcsKind::Git,
            )
            .await
            .expect("create workspace");
        let store = state
            .store_for_workspace(workspace.id)
            .await
            .expect("workspace store");
        ctx_workspace_config::update_execution_config(
            &store,
            ExecutionConfigUpdate {
                environment: ExecutionEnvironment::Sandbox,
                network_mode: Some(ContainerNetworkMode::All),
                allowlist: Some(vec!["api.example.com".to_string()]),
                image: Some("registry.example/current:v2".to_string()),
            },
        )
        .await
        .expect("update workspace execution config");

        let managed_root = temp.path().join("managed-worktree");
        std::fs::create_dir_all(&managed_root).expect("create managed root");
        let worktree = store
            .insert_worktree(Worktree {
                id: WorktreeId(Uuid::new_v4()),
                workspace_id: workspace.id,
                root_path: managed_root.to_string_lossy().to_string(),
                base_commit_sha: "abc123".to_string(),
                git_branch: Some("ctx/test".to_string()),
                vcs_kind: Some(VcsKind::Git),
                base_revision: Some("abc123".to_string()),
                vcs_ref: Some("ctx/test".to_string()),
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
            })
            .await
            .expect("insert worktree");
        let binding_snapshot = serde_json::json!({
            "mode": "sandbox",
            "container": {
                "runtime": "shared_vm_container",
                "mount_mode": "disk_isolated",
                "network_mode": "allowlist",
                "allowlist": ["github.com"],
                "image": "registry.example/snapshot:v1"
            }
        });
        store
            .upsert_sandbox_binding(SandboxBinding {
                worktree_id: worktree.id,
                workspace_id: workspace.id,
                sandbox_instance_id: ctx_core::models::sandbox_instance_id_for_workspace(
                    workspace.id,
                ),
                substrate: SandboxSubstrate::SharedVmContainer,
                guest_identity: SandboxGuestIdentity::linux_container_ubuntu(),
                profile: SandboxProfile::Standard,
                live_workspace_root: "/ctx/ws".to_string(),
                live_worktree_root: "/ctx/ws/worktrees/test".to_string(),
                execution_settings_json: Some(binding_snapshot.to_string()),
                container_name: Some("ctx-harness-test".to_string()),
                host_materialization_root: Some("/tmp/shadow".to_string()),
                created_at: Utc::now(),
            })
            .await
            .expect("insert sandbox binding");

        let resolved = resolve_existing_worktree_execution(&state, &store, &workspace, worktree.id)
            .await
            .expect("resolve worktree execution");

        assert_eq!(
            resolved.execution_environment(),
            ExecutionEnvironment::Sandbox
        );
        assert_eq!(
            resolved.effective.container.runtime,
            ContainerRuntimeKind::SharedVmContainer
        );
        assert_eq!(
            resolved.effective.container.mount_mode,
            ContainerMountMode::DiskIsolated
        );
        assert_eq!(
            resolved.effective.container.network_mode,
            ContainerNetworkMode::Allowlist
        );
        assert_eq!(
            resolved.effective.container.allowlist,
            vec!["github.com".to_string()]
        );
        assert_eq!(
            resolved.effective.container.image,
            Some("registry.example/snapshot:v1".to_string())
        );
    }
}
