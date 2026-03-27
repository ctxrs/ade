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
    if let Some(raw) = binding.execution_settings_json.as_deref() {
        return parse_sandbox_binding_execution_settings(raw)
            .context("parsing sandbox binding execution settings");
    }

    let mut settings = ExecutionSettings {
        mode: ExecutionMode::Sandbox,
        ..ExecutionSettings::default()
    };
    settings.container.runtime = crate::worktree_data_plane::binding_runtime_kind(binding);
    settings.container.mount_mode = crate::settings::ContainerMountMode::DiskIsolated;
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
    use crate::settings::{ContainerMountMode, ContainerNetworkMode, ContainerRuntimeKind};
    use chrono::Utc;
    use ctx_core::ids::{WorkspaceId, WorktreeId};
    use ctx_core::models::{SandboxProfile, SandboxRuntimeFamily};
    use uuid::Uuid;

    fn test_binding(raw: Option<String>) -> SandboxBinding {
        SandboxBinding {
            worktree_id: WorktreeId(Uuid::new_v4()),
            workspace_id: WorkspaceId(Uuid::new_v4()),
            runtime_family: SandboxRuntimeFamily::SharedVmContainer,
            profile: SandboxProfile::Standard,
            live_workspace_root: "/ctx/ws".to_string(),
            live_worktree_root: "/ctx/wt".to_string(),
            execution_settings_json: raw,
            container_name: Some("ctx-test".to_string()),
            host_projection_root: Some("/tmp/shadow".to_string()),
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
        let binding = test_binding(Some(raw.to_string()));

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
        let binding = test_binding(Some(raw.to_string()));

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
        let binding = test_binding(Some(raw.to_string()));

        let err = sandbox_execution_settings_from_binding(&binding)
            .expect_err("unknown schema version should fail");

        assert!(err
            .to_string()
            .contains("unsupported sandbox binding execution settings schema version 99"));
    }
}
