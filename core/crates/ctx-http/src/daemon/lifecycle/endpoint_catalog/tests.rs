use super::*;
use ctx_providers::adapters::{ProviderHealth, ProviderStatus, ProviderUsability};
use ctx_store::StoreManager;
use std::collections::HashMap;

fn write_invalid_harness_registry(data_root: &std::path::Path) {
    let path = data_root
        .join("providers")
        .join("harness_sources")
        .join("registry.json");
    std::fs::create_dir_all(path.parent().expect("registry parent")).unwrap();
    std::fs::write(path, "{ not valid json").unwrap();
}

#[tokio::test]
async fn endpoint_model_sweeper_counts_harness_config_load_failures() {
    let data_dir = tempfile::tempdir().unwrap();
    write_invalid_harness_registry(data_dir.path());
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    state
        .providers
        .upsert_provider_status(
            "qwen".to_string(),
            ProviderStatus {
                provider_id: "qwen".to_string(),
                installed: true,
                detected_path: None,
                version: None,
                capabilities: None,
                health: ProviderHealth::Ok,
                diagnostics: Vec::new(),
                details: HashMap::new(),
                usability: ProviderUsability::default(),
            },
        )
        .await;

    let (refreshed, failed, refreshed_provider_ids) =
        refresh_stale_selected_endpoint_model_catalogs(&state).await;

    assert_eq!(refreshed, 0);
    assert_eq!(failed, 1);
    assert!(refreshed_provider_ids.is_empty());
}
