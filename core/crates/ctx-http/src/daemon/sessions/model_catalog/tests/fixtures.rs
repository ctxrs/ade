use super::*;

pub(super) async fn test_state_with_workspace(
    data_root: &Path,
    port: u16,
) -> (Arc<AppState>, Workspace) {
    let stores = StoreManager::open(data_root).await.expect("open stores");
    let state = Arc::new(AppState::new(
        data_root.to_path_buf(),
        stores,
        HashMap::new(),
        format!("http://127.0.0.1:{port}"),
        None,
    ));
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            data_root.join("repo").to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .expect("create workspace");
    (state, workspace)
}

pub(super) async fn save_sandbox_execution_mode(state: &AppState) {
    ctx_settings_service::save_settings(
        state.global_store(),
        &Settings {
            execution: Some(ExecutionSettings {
                mode: ExecutionMode::Sandbox,
                ..ExecutionSettings::default()
            }),
            ..Settings::default()
        },
    )
    .await
    .expect("save settings");
}

pub(super) async fn seed_ready_gemini_status(state: &AppState) {
    state
        .providers
        .upsert_provider_status(
            "gemini".to_string(),
            ctx_providers::adapters::ProviderStatus {
                provider_id: "gemini".to_string(),
                installed: true,
                detected_path: None,
                version: Some("0.33.1".to_string()),
                capabilities: None,
                health: ctx_providers::adapters::ProviderHealth::Ok,
                diagnostics: Vec::new(),
                details: HashMap::new(),
                usability: ctx_providers::adapters::ProviderUsability::default(),
            },
        )
        .await;
}

pub(super) fn write_invalid_harness_registry(data_root: &Path) {
    let path = data_root
        .join("providers")
        .join("harness_sources")
        .join("registry.json");
    std::fs::create_dir_all(path.parent().expect("registry parent")).expect("mkdir registry");
    std::fs::write(path, "{ not valid json").expect("write invalid registry");
}

pub(super) fn write_invalid_agent_server_config(data_root: &Path) {
    let path = data_root
        .join("providers")
        .join("agent-servers")
        .join("agent_servers.json");
    std::fs::create_dir_all(path.parent().expect("agent server config parent"))
        .expect("mkdir agent server config");
    std::fs::write(path, "{ not valid json").expect("write invalid agent server config");
}
