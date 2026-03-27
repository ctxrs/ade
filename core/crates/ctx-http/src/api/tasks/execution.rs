use super::*;
use serde::Deserialize;
use serde_json::Value;

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
    let effective = crate::worktree_data_plane::apply_data_plane_to_execution_settings(
        &base_effective,
        &data_plane,
    )
    .context("applying worktree data plane to execution settings")?;
    Ok(ResolvedExistingWorktreeExecution {
        worktree,
        effective,
    })
}

const SANDBOX_BINDING_EXECUTION_SETTINGS_SCHEMA_V1: i64 = 1;

#[derive(Debug, Deserialize)]
struct VersionedSandboxBindingExecutionSettings {
    schema_version: i64,
    execution_settings: Value,
}

pub(crate) fn sandbox_execution_settings_from_binding(
    binding: &SandboxBinding,
) -> anyhow::Result<ExecutionSettings> {
    let substrate = crate::workspace_runtime::UbuntuSandboxSubstrate::from_binding(binding)?;
    if let Some(raw) = binding.execution_settings_json.as_deref() {
        return parse_sandbox_binding_execution_settings(raw)
            .and_then(|settings| validate_sandbox_binding_execution_settings(binding, settings))
            .context("parsing sandbox binding execution settings");
    }

    let mut settings = ExecutionSettings {
        mode: ExecutionMode::Sandbox,
        ..ExecutionSettings::default()
    };
    settings.container.runtime = substrate.runtime_kind();
    settings.container.mount_mode = crate::settings::ContainerMountMode::DiskIsolated;
    Ok(settings)
}

fn validate_sandbox_binding_execution_settings(
    binding: &SandboxBinding,
    settings: ExecutionSettings,
) -> anyhow::Result<ExecutionSettings> {
    if !matches!(settings.mode, ExecutionMode::Sandbox) {
        return Err(anyhow::anyhow!(
            "sandbox binding execution settings snapshot must keep mode=sandbox"
        ));
    }

    let expected_runtime = crate::workspace_runtime::UbuntuSandboxSubstrate::from_binding(binding)?
        .runtime_kind();
    if settings.container.runtime != expected_runtime {
        let observed = match settings.container.runtime {
            crate::settings::ContainerRuntimeKind::NativeContainer => "native_container",
            crate::settings::ContainerRuntimeKind::SharedVmContainer => "shared_vm_container",
        };
        let expected = match expected_runtime {
            crate::settings::ContainerRuntimeKind::NativeContainer => "native_container",
            crate::settings::ContainerRuntimeKind::SharedVmContainer => "shared_vm_container",
        };
        return Err(anyhow::anyhow!(
            "sandbox binding execution settings snapshot runtime {observed} does not match binding substrate {expected}"
        ));
    }

    Ok(settings)
}

fn parse_sandbox_binding_execution_settings(raw: &str) -> anyhow::Result<ExecutionSettings> {
    let value: Value =
        serde_json::from_str(raw).context("parsing sandbox binding execution settings JSON")?;
    parse_sandbox_binding_execution_settings_value(value)
}

fn parse_sandbox_binding_execution_settings_value(
    value: Value,
) -> anyhow::Result<ExecutionSettings> {
    match value {
        Value::Object(map) if map.contains_key("schema_version") => {
            let versioned: VersionedSandboxBindingExecutionSettings =
                serde_json::from_value(Value::Object(map))
                    .context("parsing versioned sandbox binding execution settings")?;
            match versioned.schema_version {
                SANDBOX_BINDING_EXECUTION_SETTINGS_SCHEMA_V1 => {
                    parse_and_normalize_execution_settings(versioned.execution_settings)
                }
                other => Err(anyhow::anyhow!(
                    "unsupported sandbox binding execution settings schema version {other}"
                )),
            }
        }
        Value::Object(map) => parse_and_normalize_execution_settings(Value::Object(map)),
        other => Err(anyhow::anyhow!(
            "sandbox binding execution settings snapshot must be a JSON object, found {other}"
        )),
    }
}

fn parse_and_normalize_execution_settings(value: Value) -> anyhow::Result<ExecutionSettings> {
    let mut settings: ExecutionSettings =
        serde_json::from_value(value).context("parsing execution settings payload")?;
    crate::settings::normalize_container_execution_settings(&mut settings.container);
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::AppState;
    use crate::settings::{ContainerMountMode, ContainerNetworkMode, ContainerRuntimeKind};
    use crate::workspace_config::{self, ExecutionConfigUpdate};
    use chrono::Utc;
    use ctx_core::ids::{WorkspaceId, WorktreeId};
    use ctx_core::models::{
        ExecutionEnvironment, SandboxGuestIdentity, SandboxProfile, SandboxSubstrate, VcsKind,
    };
    use ctx_store::StoreManager;
    use std::collections::HashMap;
    use std::sync::Arc;
    use uuid::Uuid;

    fn test_binding(substrate: SandboxSubstrate, raw: Option<String>) -> SandboxBinding {
        SandboxBinding {
            worktree_id: WorktreeId(Uuid::new_v4()),
            workspace_id: WorkspaceId(Uuid::new_v4()),
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
        let binding = test_binding(
            SandboxSubstrate::SharedVmContainer,
            Some(raw.to_string()),
        );

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
        let binding = test_binding(
            SandboxSubstrate::SharedVmContainer,
            Some(raw.to_string()),
        );

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
        workspace_config::update_execution_config(
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
