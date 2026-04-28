use ctx_core::ids::WorkspaceId;
use ctx_core::models::ExecutionEnvironment as SessionExecutionEnvironment;
use ctx_workspace_config as workspace_config;

use crate::daemon::AppState;
use crate::execution_policy::{ExecutionPolicyDenied, HostExecutionPolicy};
use crate::settings;
use crate::settings::ExecutionSettings;
use crate::settings::{ContainerNetworkMode, ExecutionMode};
use ctx_provider_install::install_state::InstallTarget;

#[derive(Debug)]
pub enum EffectiveExecutionSettingsError {
    InvalidWorkspaceOverride(anyhow::Error),
    Internal(anyhow::Error),
}

impl EffectiveExecutionSettingsError {
    pub fn into_inner(self) -> anyhow::Error {
        match self {
            Self::InvalidWorkspaceOverride(err) | Self::Internal(err) => err,
        }
    }
}

pub async fn effective_execution_settings_classified(
    state: &AppState,
    workspace_id: WorkspaceId,
) -> Result<ExecutionSettings, EffectiveExecutionSettingsError> {
    let settings_data = settings::load_settings(state.global_store())
        .await
        .map_err(EffectiveExecutionSettingsError::Internal)?;
    let mut effective = settings_data.execution.clone().unwrap_or_default();
    let store = state
        .store_for_workspace(workspace_id)
        .await
        .map_err(EffectiveExecutionSettingsError::Internal)?;
    if let Some(ov) = workspace_config::load_execution_settings_override(&store)
        .await
        .map_err(EffectiveExecutionSettingsError::InvalidWorkspaceOverride)?
    {
        apply_workspace_execution_settings_override(&mut effective, &ov)
            .map_err(EffectiveExecutionSettingsError::InvalidWorkspaceOverride)?;
    }
    HostExecutionPolicy::current()
        .and_then(|policy| policy.validate_execution_settings(&effective))
        .map_err(EffectiveExecutionSettingsError::InvalidWorkspaceOverride)?;
    Ok(effective)
}

pub(crate) fn apply_workspace_execution_settings_override(
    settings: &mut ExecutionSettings,
    ov: &workspace_config::ExecutionSettingsOverride,
) -> anyhow::Result<()> {
    let ov = normalize_persisted_workspace_execution_settings_override(settings, ov)?;
    validate_workspace_execution_settings_override(settings, &ov)?;
    workspace_config::apply_execution_settings_override(settings, &ov);
    Ok(())
}

fn normalize_persisted_workspace_execution_settings_override(
    _settings: &ExecutionSettings,
    ov: &workspace_config::ExecutionSettingsOverride,
) -> anyhow::Result<workspace_config::ExecutionSettingsOverride> {
    let mut normalized = ov.clone();
    // Persisted host overrides predate the daemon-owned sandbox-only gate. Treat them as stale
    // reads; new host writes still go through strict validation before persistence.
    if matches!(
        HostExecutionPolicy::current()?,
        HostExecutionPolicy::SandboxOnly
    ) && matches!(normalized.mode, Some(ExecutionMode::Host))
    {
        normalized.mode = Some(ExecutionMode::Sandbox);
        normalized.container = workspace_config::ContainerExecutionSettingsOverride::default();
    }
    Ok(normalized)
}

pub(crate) fn validate_workspace_execution_settings_override(
    settings: &ExecutionSettings,
    ov: &workspace_config::ExecutionSettingsOverride,
) -> anyhow::Result<()> {
    if matches!(settings.mode, ExecutionMode::Sandbox) {
        if matches!(ov.mode, Some(ExecutionMode::Host)) {
            return Err(ExecutionPolicyDenied::new(
                "workspace execution override cannot select host when daemon execution mode is sandbox"
            )
            .into());
        }
        validate_sandbox_network_override(settings, ov)?;
    }
    let mut effective = settings.clone();
    workspace_config::apply_execution_settings_override(&mut effective, ov);
    HostExecutionPolicy::current()?.validate_execution_settings(&effective)?;
    Ok(())
}

fn validate_sandbox_network_override(
    settings: &ExecutionSettings,
    ov: &workspace_config::ExecutionSettingsOverride,
) -> anyhow::Result<()> {
    let requested_network = ov
        .container
        .network_mode
        .as_ref()
        .unwrap_or(&settings.container.network_mode);
    match (&settings.container.network_mode, requested_network) {
        (ContainerNetworkMode::LlmOnly, ContainerNetworkMode::LlmOnly) => Ok(()),
        (ContainerNetworkMode::LlmOnly, requested) => {
            Err(ExecutionPolicyDenied::new(format!(
                "workspace execution override cannot broaden sandbox network mode from llm_only to {}",
                network_mode_label(requested)
            ))
            .into())
        }
        (ContainerNetworkMode::Allowlist, ContainerNetworkMode::All) => {
            Err(ExecutionPolicyDenied::new(
                "workspace execution override cannot broaden sandbox network mode from allowlist to all"
            )
            .into())
        }
        (ContainerNetworkMode::Allowlist, ContainerNetworkMode::Allowlist) => {
            validate_allowlist_subset(&settings.container.allowlist, &ov.container.allowlist)
        }
        (ContainerNetworkMode::Allowlist, ContainerNetworkMode::LlmOnly) => Ok(()),
        (ContainerNetworkMode::All, _) => Ok(()),
    }
}

fn validate_allowlist_subset(
    daemon_allowlist: &[String],
    workspace_allowlist: &Option<Vec<String>>,
) -> anyhow::Result<()> {
    let Some(workspace_allowlist) = workspace_allowlist else {
        return Ok(());
    };
    let allowed = daemon_allowlist
        .iter()
        .filter_map(|value| trimmed_nonempty(value))
        .collect::<std::collections::BTreeSet<_>>();
    for entry in workspace_allowlist
        .iter()
        .filter_map(|value| trimmed_nonempty(value))
    {
        if !allowed.contains(&entry) {
            return Err(ExecutionPolicyDenied::new(format!(
                "workspace execution allowlist entry `{entry}` is not allowed by daemon sandbox allowlist"
            ))
            .into());
        }
    }
    Ok(())
}

fn trimmed_nonempty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn network_mode_label(mode: &ContainerNetworkMode) -> &'static str {
    match mode {
        ContainerNetworkMode::LlmOnly => "llm_only",
        ContainerNetworkMode::Allowlist => "allowlist",
        ContainerNetworkMode::All => "all",
    }
}

/// Compute effective execution settings for a workspace, combining daemon defaults with any
/// workspace runtime override.
pub async fn effective_execution_settings(
    state: &AppState,
    workspace_id: WorkspaceId,
) -> anyhow::Result<ExecutionSettings> {
    effective_execution_settings_classified(state, workspace_id)
        .await
        .map_err(EffectiveExecutionSettingsError::into_inner)
}

pub fn install_target_for_settings(settings: &ExecutionSettings) -> InstallTarget {
    if matches!(settings.mode, crate::settings::ExecutionMode::Sandbox) {
        InstallTarget::Container
    } else {
        InstallTarget::Host
    }
}

pub async fn effective_install_target(
    state: &AppState,
    workspace_id: WorkspaceId,
) -> anyhow::Result<InstallTarget> {
    let effective = effective_execution_settings(state, workspace_id).await?;
    Ok(install_target_for_settings(&effective))
}

pub fn apply_execution_environment(
    settings: &mut ExecutionSettings,
    execution_environment: SessionExecutionEnvironment,
) {
    match execution_environment {
        SessionExecutionEnvironment::Host => {
            settings.mode = crate::settings::ExecutionMode::Host;
        }
        SessionExecutionEnvironment::Sandbox => {
            settings.mode = crate::settings::ExecutionMode::Sandbox;
            settings.container.mount_mode = crate::settings::ContainerMountMode::DiskIsolated;
        }
    }
}

pub(crate) fn validate_execution_environment_against_settings(
    settings: &ExecutionSettings,
    execution_environment: SessionExecutionEnvironment,
) -> anyhow::Result<()> {
    HostExecutionPolicy::current()?.validate_execution_environment(execution_environment)?;
    if matches!(settings.mode, ExecutionMode::Sandbox)
        && matches!(execution_environment, SessionExecutionEnvironment::Host)
    {
        return Err(ExecutionPolicyDenied::new(
            "session execution environment host is not allowed when effective daemon execution mode is sandbox"
        )
        .into());
    }
    Ok(())
}

pub async fn effective_execution_settings_for_environment(
    state: &AppState,
    workspace_id: WorkspaceId,
    execution_environment: SessionExecutionEnvironment,
) -> anyhow::Result<ExecutionSettings> {
    let mut effective = effective_execution_settings(state, workspace_id).await?;
    validate_execution_environment_against_settings(&effective, execution_environment)?;
    apply_execution_environment(&mut effective, execution_environment);
    Ok(effective)
}

pub async fn effective_install_target_for_environment(
    state: &AppState,
    workspace_id: WorkspaceId,
    execution_environment: SessionExecutionEnvironment,
) -> anyhow::Result<InstallTarget> {
    let effective =
        effective_execution_settings_for_environment(state, workspace_id, execution_environment)
            .await?;
    Ok(install_target_for_settings(&effective))
}

#[cfg(test)]
mod tests {
    use super::{
        effective_execution_settings, effective_execution_settings_classified,
        effective_install_target, install_target_for_settings, SessionExecutionEnvironment,
    };

    use std::collections::HashMap;
    use std::path::Path;

    use ctx_core::ids::WorkspaceId;
    use ctx_core::models::{VcsKind, Workspace};
    use ctx_store::StoreManager;
    use ctx_workspace_config::{ExecutionConfigUpdate, ExecutionEnvironment};

    use crate::daemon::AppState;
    use crate::execution_policy::EXECUTION_POLICY_TEST_ENV_LOCK;
    use crate::settings::{self, ContainerNetworkMode, ExecutionMode, ExecutionSettings, Settings};
    use ctx_provider_install::install_state::InstallTarget;

    static STORE_MANAGER_OPEN_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, previous }
        }

        fn remove(key: &'static str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(value) = &self.previous {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    struct ExecutionEnvGuards {
        _lock: tokio::sync::MutexGuard<'static, ()>,
        _policy: EnvVarGuard,
        _mode: EnvVarGuard,
    }

    async fn clean_execution_env() -> ExecutionEnvGuards {
        let lock = EXECUTION_POLICY_TEST_ENV_LOCK.lock().await;
        ExecutionEnvGuards {
            _lock: lock,
            _policy: EnvVarGuard::remove("CTX_HOST_EXECUTION_POLICY"),
            _mode: EnvVarGuard::remove("CTX_EXECUTION_MODE"),
        }
    }

    async fn sandbox_only_execution_env() -> ExecutionEnvGuards {
        let lock = EXECUTION_POLICY_TEST_ENV_LOCK.lock().await;
        ExecutionEnvGuards {
            _lock: lock,
            _policy: EnvVarGuard::set("CTX_HOST_EXECUTION_POLICY", "sandbox_only"),
            _mode: EnvVarGuard::remove("CTX_EXECUTION_MODE"),
        }
    }

    async fn open_store_manager(path: &Path) -> StoreManager {
        let _guard = STORE_MANAGER_OPEN_LOCK.lock().await;
        StoreManager::open(path).await.expect("open stores")
    }

    #[test]
    fn install_target_for_settings_matches_execution_mode() {
        let host = ExecutionSettings {
            mode: ExecutionMode::Host,
            ..ExecutionSettings::default()
        };
        let container = ExecutionSettings {
            mode: ExecutionMode::Sandbox,
            ..ExecutionSettings::default()
        };

        assert_eq!(install_target_for_settings(&host), InstallTarget::Host);
        assert_eq!(
            install_target_for_settings(&container),
            InstallTarget::Container
        );
    }

    #[tokio::test]
    async fn effective_install_target_errors_for_missing_workspace() {
        let _env = clean_execution_env().await;
        let temp = tempfile::tempdir().expect("tempdir");
        let stores = open_store_manager(temp.path()).await;
        let state = AppState::new(
            temp.path().to_path_buf(),
            stores,
            HashMap::new(),
            "http://127.0.0.1:4310".to_string(),
            None,
        );

        let err = effective_install_target(&state, WorkspaceId::new())
            .await
            .expect_err("missing workspace should fail");
        let message = format!("{err:#}");
        assert!(message.contains("workspace"));
        assert!(message.contains("not found"));
    }

    async fn state_with_workspace() -> (tempfile::TempDir, AppState, Workspace) {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("create repo root");
        let stores = open_store_manager(temp.path()).await;
        let state = AppState::new(
            temp.path().to_path_buf(),
            stores,
            HashMap::new(),
            "http://127.0.0.1:4310".to_string(),
            None,
        );
        let workspace = state
            .global_store()
            .create_workspace(
                "ws".to_string(),
                repo_root.to_string_lossy().to_string(),
                VcsKind::Git,
            )
            .await
            .expect("create workspace");
        (temp, state, workspace)
    }

    async fn set_daemon_execution_settings(state: &AppState, execution: ExecutionSettings) {
        settings::save_settings(
            state.global_store(),
            &Settings {
                execution: Some(execution),
                ..Settings::default()
            },
        )
        .await
        .expect("save daemon settings");
    }

    #[tokio::test]
    async fn workspace_override_cannot_broaden_daemon_sandbox_to_host() {
        let _env = clean_execution_env().await;
        let (_temp, state, workspace) = state_with_workspace().await;
        set_daemon_execution_settings(
            &state,
            ExecutionSettings {
                mode: ExecutionMode::Sandbox,
                ..ExecutionSettings::default()
            },
        )
        .await;
        let store = state
            .store_for_workspace(workspace.id)
            .await
            .expect("workspace store");
        ctx_workspace_config::update_execution_config(
            &store,
            ExecutionConfigUpdate {
                environment: ExecutionEnvironment::Host,
                network_mode: None,
                allowlist: None,
                image: None,
            },
        )
        .await
        .expect("write workspace override");

        let err = effective_execution_settings_classified(&state, workspace.id)
            .await
            .expect_err("workspace host override must not broaden daemon sandbox policy");
        let message = format!("{:#}", err.into_inner());
        assert!(message.contains("cannot select host"));
    }

    #[tokio::test]
    async fn sandbox_only_policy_normalizes_persisted_workspace_host_override_to_sandbox() {
        let _env = sandbox_only_execution_env().await;
        let (_temp, state, workspace) = state_with_workspace().await;
        let store = state
            .store_for_workspace(workspace.id)
            .await
            .expect("workspace store");
        ctx_workspace_config::update_execution_config(
            &store,
            ExecutionConfigUpdate {
                environment: ExecutionEnvironment::Host,
                network_mode: None,
                allowlist: None,
                image: None,
            },
        )
        .await
        .expect("write workspace override");

        let effective = effective_execution_settings(&state, workspace.id)
            .await
            .expect("sandbox-only policy should constrain persisted host override to sandbox");

        assert_eq!(effective.mode, ExecutionMode::Sandbox);
    }

    #[tokio::test]
    async fn sandbox_only_policy_drops_stale_container_fields_from_persisted_workspace_host_override(
    ) {
        let _env = sandbox_only_execution_env().await;
        let (_temp, state, workspace) = state_with_workspace().await;
        set_daemon_execution_settings(
            &state,
            ExecutionSettings {
                mode: ExecutionMode::Sandbox,
                ..ExecutionSettings::default()
            },
        )
        .await;
        let store = state
            .store_for_workspace(workspace.id)
            .await
            .expect("workspace store");
        ctx_workspace_config::update_execution_config(
            &store,
            ExecutionConfigUpdate {
                environment: ExecutionEnvironment::Host,
                network_mode: Some(ContainerNetworkMode::All),
                allowlist: Some(vec!["example.com".to_string()]),
                image: Some("ignored.example/legacy-host".to_string()),
            },
        )
        .await
        .expect("write workspace override");

        let effective = effective_execution_settings(&state, workspace.id)
            .await
            .expect("stale host override container fields should be ignored under sandbox-only");

        assert_eq!(effective.mode, ExecutionMode::Sandbox);
        assert_eq!(
            effective.container.network_mode,
            ContainerNetworkMode::LlmOnly
        );
        assert!(effective.container.allowlist.is_empty());
        assert_eq!(effective.container.image, None);
    }

    #[tokio::test]
    async fn sandbox_only_policy_rejects_new_workspace_host_override() {
        let _env = sandbox_only_execution_env().await;
        let base = ExecutionSettings::default();
        let override_config = ctx_workspace_config::ExecutionSettingsOverride {
            mode: Some(ExecutionMode::Host),
            ..Default::default()
        };

        let err = super::validate_workspace_execution_settings_override(&base, &override_config)
            .expect_err("new workspace host override must be rejected");
        let message = format!("{err:#}");
        assert!(message.contains("host execution is disabled by daemon policy"));
    }

    #[tokio::test]
    async fn sandbox_only_policy_rejects_ctx_execution_mode_host_override() {
        let _env_guard = EXECUTION_POLICY_TEST_ENV_LOCK.lock().await;
        let _policy = EnvVarGuard::set("CTX_HOST_EXECUTION_POLICY", "sandbox_only");
        let _mode = EnvVarGuard::set("CTX_EXECUTION_MODE", "host");
        let (_temp, state, workspace) = state_with_workspace().await;

        let err = effective_execution_settings_classified(&state, workspace.id)
            .await
            .expect_err("sandbox-only policy must reject CTX_EXECUTION_MODE=host");
        let message = format!("{:#}", err.into_inner());
        assert!(message.contains("CTX_EXECUTION_MODE=host is disabled"));
    }

    #[tokio::test]
    async fn sandbox_only_policy_normalizes_existing_host_default_to_sandbox() {
        let _env = sandbox_only_execution_env().await;
        let (_temp, state, workspace) = state_with_workspace().await;
        set_daemon_execution_settings(
            &state,
            ExecutionSettings {
                mode: ExecutionMode::Host,
                ..ExecutionSettings::default()
            },
        )
        .await;

        let effective = effective_execution_settings(&state, workspace.id)
            .await
            .expect("sandbox-only policy should repair stored host default at read time");

        assert_eq!(effective.mode, ExecutionMode::Sandbox);
    }

    #[tokio::test]
    async fn sandbox_only_policy_drops_stale_container_fields_from_existing_host_default() {
        let _env = sandbox_only_execution_env().await;
        let (_temp, state, workspace) = state_with_workspace().await;
        set_daemon_execution_settings(
            &state,
            ExecutionSettings {
                mode: ExecutionMode::Host,
                container: crate::settings::ContainerExecutionSettings {
                    network_mode: ContainerNetworkMode::All,
                    allowlist: vec!["example.com".to_string()],
                    image: Some("ignored.example/legacy-host".to_string()),
                    ..Default::default()
                },
            },
        )
        .await;

        let effective = effective_execution_settings(&state, workspace.id)
            .await
            .expect("sandbox-only policy should ignore stale host default container fields");

        assert_eq!(effective.mode, ExecutionMode::Sandbox);
        assert_eq!(
            effective.container.network_mode,
            ContainerNetworkMode::LlmOnly
        );
        assert!(effective.container.allowlist.is_empty());
        assert_eq!(effective.container.image, None);
    }

    #[tokio::test]
    async fn workspace_override_can_restrict_daemon_host_to_sandbox() {
        let _env = clean_execution_env().await;
        let (_temp, state, workspace) = state_with_workspace().await;
        let store = state
            .store_for_workspace(workspace.id)
            .await
            .expect("workspace store");
        ctx_workspace_config::update_execution_config(
            &store,
            ExecutionConfigUpdate {
                environment: ExecutionEnvironment::Sandbox,
                network_mode: Some(ContainerNetworkMode::All),
                allowlist: None,
                image: None,
            },
        )
        .await
        .expect("write workspace override");

        let effective = effective_execution_settings(&state, workspace.id)
            .await
            .expect("workspace may restrict host default to sandbox");
        assert_eq!(effective.mode, ExecutionMode::Sandbox);
        assert_eq!(effective.container.network_mode, ContainerNetworkMode::All);
    }

    #[tokio::test]
    async fn workspace_override_cannot_broaden_daemon_sandbox_network_mode() {
        let _env = clean_execution_env().await;
        let (_temp, state, workspace) = state_with_workspace().await;
        set_daemon_execution_settings(
            &state,
            ExecutionSettings {
                mode: ExecutionMode::Sandbox,
                ..ExecutionSettings::default()
            },
        )
        .await;
        let store = state
            .store_for_workspace(workspace.id)
            .await
            .expect("workspace store");
        ctx_workspace_config::update_execution_config(
            &store,
            ExecutionConfigUpdate {
                environment: ExecutionEnvironment::Sandbox,
                network_mode: Some(ContainerNetworkMode::All),
                allowlist: None,
                image: None,
            },
        )
        .await
        .expect("write workspace override");

        let err = effective_execution_settings_classified(&state, workspace.id)
            .await
            .expect_err("workspace network override must not broaden daemon sandbox policy");
        let message = format!("{:#}", err.into_inner());
        assert!(message.contains("cannot broaden sandbox network mode"));
    }

    #[tokio::test]
    async fn workspace_allowlist_override_must_be_subset_of_daemon_allowlist() {
        let _env = clean_execution_env().await;
        let (_temp, state, workspace) = state_with_workspace().await;
        set_daemon_execution_settings(
            &state,
            ExecutionSettings {
                mode: ExecutionMode::Sandbox,
                container: crate::settings::ContainerExecutionSettings {
                    network_mode: ContainerNetworkMode::Allowlist,
                    allowlist: vec!["api.openai.com".to_string()],
                    ..Default::default()
                },
            },
        )
        .await;
        let store = state
            .store_for_workspace(workspace.id)
            .await
            .expect("workspace store");
        ctx_workspace_config::update_execution_config(
            &store,
            ExecutionConfigUpdate {
                environment: ExecutionEnvironment::Sandbox,
                network_mode: Some(ContainerNetworkMode::Allowlist),
                allowlist: Some(vec![
                    "api.openai.com".to_string(),
                    "example.com".to_string(),
                ]),
                image: None,
            },
        )
        .await
        .expect("write workspace override");

        let err = effective_execution_settings_classified(&state, workspace.id)
            .await
            .expect_err("workspace allowlist must not broaden daemon sandbox allowlist");
        let message = format!("{:#}", err.into_inner());
        assert!(message.contains("example.com"));
        assert!(message.contains("daemon sandbox allowlist"));
    }

    #[tokio::test]
    async fn persisted_host_session_cannot_broaden_daemon_sandbox_policy() {
        let _env = clean_execution_env().await;
        let (_temp, state, workspace) = state_with_workspace().await;
        set_daemon_execution_settings(
            &state,
            ExecutionSettings {
                mode: ExecutionMode::Sandbox,
                ..ExecutionSettings::default()
            },
        )
        .await;

        let err = super::effective_execution_settings_for_environment(
            &state,
            workspace.id,
            SessionExecutionEnvironment::Host,
        )
        .await
        .expect_err("persisted host session must not broaden daemon sandbox policy");
        let message = format!("{err:#}");
        assert!(message.contains("host is not allowed"));
        assert!(crate::execution_policy::is_execution_policy_denial(&err));
    }

    #[tokio::test]
    async fn persisted_sandbox_session_can_restrict_daemon_host_policy() {
        let _env = clean_execution_env().await;
        let (_temp, state, workspace) = state_with_workspace().await;

        let effective = super::effective_execution_settings_for_environment(
            &state,
            workspace.id,
            SessionExecutionEnvironment::Sandbox,
        )
        .await
        .expect("persisted sandbox session may restrict host default");

        assert_eq!(effective.mode, ExecutionMode::Sandbox);
    }

    #[tokio::test]
    async fn sandbox_only_policy_allows_persisted_sandbox_session_with_existing_host_default() {
        let _env = sandbox_only_execution_env().await;
        let (_temp, state, workspace) = state_with_workspace().await;
        set_daemon_execution_settings(
            &state,
            ExecutionSettings {
                mode: ExecutionMode::Host,
                ..ExecutionSettings::default()
            },
        )
        .await;

        let effective = super::effective_execution_settings_for_environment(
            &state,
            workspace.id,
            SessionExecutionEnvironment::Sandbox,
        )
        .await
        .expect("persisted sandbox session should survive sandbox-only policy enablement");

        assert_eq!(effective.mode, ExecutionMode::Sandbox);
    }
}
