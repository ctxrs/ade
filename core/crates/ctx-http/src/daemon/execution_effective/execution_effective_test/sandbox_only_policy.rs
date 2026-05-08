use super::*;

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
async fn sandbox_only_policy_drops_stale_container_fields_from_persisted_workspace_host_override() {
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
            container: ctx_settings_model::ContainerExecutionSettings {
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
