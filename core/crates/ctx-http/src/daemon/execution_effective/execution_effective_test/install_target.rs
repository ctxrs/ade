use super::*;

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
