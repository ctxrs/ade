use super::{load_provider_model_catalog, resolve_model_id, ModelCatalog};

use std::collections::HashMap;
use std::sync::Arc;

use ctx_core::models::VcsKind;
use ctx_store::StoreManager;

use crate::daemon::AppState;
use crate::settings::{ExecutionMode, ExecutionSettings, Settings};

fn write_invalid_harness_registry(data_root: &std::path::Path) {
    let path = data_root
        .join("providers")
        .join("harness_sources")
        .join("registry.json");
    std::fs::create_dir_all(path.parent().expect("registry parent")).expect("mkdir registry");
    std::fs::write(path, "{ not valid json").expect("write invalid registry");
}

fn write_invalid_agent_server_config(data_root: &std::path::Path) {
    let path = data_root
        .join("providers")
        .join("agent-servers")
        .join("agent_servers.json");
    std::fs::create_dir_all(path.parent().expect("agent server config parent"))
        .expect("mkdir agent server config");
    std::fs::write(path, "{ not valid json").expect("write invalid agent server config");
}

#[tokio::test]
async fn load_provider_model_catalog_reads_target_scoped_options_cache() {
    let temp = tempfile::tempdir().expect("tempdir");
    let stores = StoreManager::open(temp.path()).await.expect("open stores");
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4310".to_string(),
        None,
    ));
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            temp.path().join("repo").to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .expect("create workspace");
    crate::settings::save_settings(
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

    state.providers.options_cache.lock().await.insert(
        format!("{}/container/codex", workspace.id.0),
        crate::daemon::CachedProviderOptions {
            cached_at: std::time::Instant::now(),
            value: serde_json::json!({
                "models": {
                    "models": [
                        { "id": "gpt-5" },
                        { "id": "gpt-5/high" }
                    ],
                    "current_model_id": "gpt-5",
                    "meta": {
                        "source_kind": "subscription",
                        "catalog_source": "runtime_probe_live",
                        "refresh_pending": false
                    }
                }
            }),
        },
    );

    let catalog = load_provider_model_catalog(&state, &workspace, "codex")
        .await
        .expect("load catalog")
        .expect("catalog");

    assert!(catalog.full_ids.iter().any(|id| id == "gpt-5"));
    assert!(catalog.full_ids.iter().any(|id| id == "gpt-5/high"));
    assert_eq!(catalog.current_model_id.as_deref(), Some("gpt-5"));
}

#[tokio::test]
async fn load_provider_model_catalog_falls_back_to_pinned_gemini_catalog() {
    let temp = tempfile::tempdir().expect("tempdir");
    let stores = StoreManager::open(temp.path()).await.expect("open stores");
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4311".to_string(),
        None,
    ));
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            temp.path().join("repo").to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .expect("create workspace");

    state.providers.statuses.lock().await.insert(
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
    );

    let catalog = load_provider_model_catalog(&state, &workspace, "gemini")
        .await
        .expect("load catalog")
        .expect("catalog");

    assert_eq!(catalog.current_model_id.as_deref(), Some("auto-gemini-3"));
    assert!(catalog.full_ids.iter().any(|id| id == "auto-gemini-3"));
    assert!(catalog
        .full_ids
        .iter()
        .any(|id| id == "gemini-3-pro-preview"));
}

#[tokio::test]
async fn load_provider_model_catalog_surfaces_harness_config_errors() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_invalid_harness_registry(temp.path());
    let stores = StoreManager::open(temp.path()).await.expect("open stores");
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4312".to_string(),
        None,
    ));
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            temp.path().join("repo").to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .expect("create workspace");

    let err = load_provider_model_catalog(&state, &workspace, "qwen")
        .await
        .expect_err("harness config error should surface");
    assert!(err.contains("parsing harness source registry"));
}

#[tokio::test]
async fn load_provider_model_catalog_surfaces_agent_server_config_errors() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_invalid_agent_server_config(temp.path());
    let stores = StoreManager::open(temp.path()).await.expect("open stores");
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4313".to_string(),
        None,
    ));
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            temp.path().join("repo").to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .expect("create workspace");

    let err = load_provider_model_catalog(&state, &workspace, "qwen")
        .await
        .expect_err("managed config error should surface");
    assert!(err.contains("parsing agent server config"));
}

#[tokio::test]
async fn load_provider_model_catalog_surfaces_agent_server_config_errors_for_pinned_catalogs() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_invalid_agent_server_config(temp.path());
    let stores = StoreManager::open(temp.path()).await.expect("open stores");
    let state = Arc::new(AppState::new(
        temp.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4314".to_string(),
        None,
    ));
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            temp.path().join("repo").to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .expect("create workspace");

    let err = load_provider_model_catalog(&state, &workspace, "gemini")
        .await
        .expect_err("managed config error should surface for pinned catalogs too");
    assert!(err.contains("parsing agent server config"));
}

#[test]
fn model_catalog_default_model_id_prefers_current_model_id() {
    let catalog = ModelCatalog {
        full_ids: vec!["gemini-2.5-pro".to_string(), "auto-gemini-3".to_string()],
        current_model_id: Some("auto-gemini-3".to_string()),
        base_ids: Vec::new(),
        efforts_by_base: HashMap::new(),
        full_id_by_base_effort: HashMap::new(),
        info_by_full_id: HashMap::new(),
    };

    assert_eq!(catalog.default_model_id(), Some("auto-gemini-3"));
}

#[test]
fn resolve_model_id_allows_explicit_unknown_model_when_catalog_is_present() {
    let catalog = ModelCatalog {
        full_ids: vec!["gemini-2.5-pro".to_string(), "gemini-2.5-flash".to_string()],
        current_model_id: Some("gemini-2.5-pro".to_string()),
        base_ids: vec!["gemini-2.5-pro".to_string(), "gemini-2.5-flash".to_string()],
        efforts_by_base: HashMap::new(),
        full_id_by_base_effort: HashMap::new(),
        info_by_full_id: HashMap::new(),
    };

    let resolved = resolve_model_id(Some("gemini-3-pro-exp"), None, None, Some(&catalog))
        .expect("explicit unknown model should be accepted");

    assert_eq!(resolved.model_id, "gemini-3-pro-exp");
    assert_eq!(resolved.reasoning_effort, None);
    assert_eq!(resolved.full_model_id, "gemini-3-pro-exp");
}

#[test]
fn resolve_model_id_allows_explicit_unknown_model_with_reasoning_effort() {
    let catalog = ModelCatalog {
        full_ids: vec!["gpt-5/medium".to_string(), "gpt-5/high".to_string()],
        current_model_id: Some("gpt-5/medium".to_string()),
        base_ids: vec!["gpt-5".to_string()],
        efforts_by_base: HashMap::from([(
            "gpt-5".to_string(),
            vec!["medium".to_string(), "high".to_string()],
        )]),
        full_id_by_base_effort: HashMap::from([(
            "gpt-5".to_string(),
            HashMap::from([
                ("medium".to_string(), "gpt-5/medium".to_string()),
                ("high".to_string(), "gpt-5/high".to_string()),
            ]),
        )]),
        info_by_full_id: HashMap::new(),
    };

    let resolved = resolve_model_id(Some("gpt-6"), Some("xhigh"), None, Some(&catalog))
        .expect("explicit unknown model with effort should be accepted");

    assert_eq!(resolved.model_id, "gpt-6");
    assert_eq!(resolved.reasoning_effort.as_deref(), Some("xhigh"));
    assert_eq!(resolved.full_model_id, "gpt-6/xhigh");
}
