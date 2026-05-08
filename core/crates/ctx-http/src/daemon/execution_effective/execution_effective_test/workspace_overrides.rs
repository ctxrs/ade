use super::*;

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
            container: ctx_settings_model::ContainerExecutionSettings {
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
