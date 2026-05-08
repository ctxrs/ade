use super::*;

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
    assert!(ctx_settings_service::is_execution_policy_denial(&err));
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
