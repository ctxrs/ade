use super::*;
use ctx_providers::adapters::{ProviderHealth, ProviderUsability};

fn write_invalid_agent_server_config(data_root: &std::path::Path) {
    let path = data_root
        .join("providers")
        .join("agent-servers")
        .join("agent_servers.json");
    std::fs::create_dir_all(path.parent().expect("agent server config parent")).unwrap();
    std::fs::write(path, "{ not valid json").unwrap();
}

#[tokio::test]
async fn health_and_diagnostics_include_storage_guard_state() {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    state.core.storage_guard.publish(StorageGuardStatus {
        level: StorageGuardLevel::Warning,
        reserve_file_active: true,
        active: Some(StorageGuardPathStatus {
            label: "CTX data root".to_string(),
            path: data_dir.path().to_string_lossy().to_string(),
            mount_point: "/".to_string(),
            free_bytes: 1_800_000_000,
            total_bytes: 10_000_000_000,
        }),
        ..StorageGuardStatus::default()
    });

    let app = api::router(state);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/health")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let health: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        health
            .pointer("/storage/level")
            .and_then(serde_json::Value::as_str),
        Some("warning"),
        "expected storage warning state in health payload: {health:#?}"
    );
    assert_eq!(
        health
            .pointer("/storage/active/label")
            .and_then(serde_json::Value::as_str),
        Some("CTX data root"),
        "expected active storage path in health payload: {health:#?}"
    );

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/diagnostics")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let diagnostics: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        diagnostics
            .pointer("/daemon/storage/level")
            .and_then(serde_json::Value::as_str),
        Some("warning"),
        "expected storage warning state in diagnostics payload: {diagnostics:#?}"
    );
    assert_eq!(
        diagnostics
            .pointer("/daemon/storage/active/path")
            .and_then(serde_json::Value::as_str),
        Some(data_dir.path().to_string_lossy().as_ref()),
        "expected active storage path in diagnostics payload: {diagnostics:#?}"
    );
}

#[tokio::test]
async fn diagnostics_marks_provider_statuses_with_agent_server_config_errors() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    write_invalid_agent_server_config(data_dir.path());
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    state.providers.statuses.lock().await.insert(
        "qwen".to_string(),
        ProviderStatus {
            provider_id: "qwen".to_string(),
            installed: true,
            detected_path: None,
            version: Some("0.1.0".to_string()),
            capabilities: None,
            health: ProviderHealth::Ok,
            diagnostics: Vec::new(),
            details: HashMap::new(),
            usability: ProviderUsability::default(),
        },
    );
    let app = api::router(state);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/diagnostics")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let diagnostics: serde_json::Value = serde_json::from_slice(&body).unwrap();

    let qwen = diagnostics["providers"]
        .as_array()
        .and_then(|providers| {
            providers
                .iter()
                .find(|provider| provider["provider_id"].as_str() == Some("qwen"))
        })
        .unwrap_or_else(|| panic!("missing qwen provider diagnostics: {diagnostics:#?}"));

    assert_eq!(qwen["health"].as_str(), Some("error"));
    assert_eq!(
        qwen.pointer("/usability/reason_code")
            .and_then(serde_json::Value::as_str),
        Some("managed_config_error")
    );
    assert_eq!(
        qwen.pointer("/details/managed_config_error")
            .and_then(serde_json::Value::as_str),
        Some("true")
    );
    assert!(qwen["diagnostics"]
        .as_array()
        .is_some_and(|values| values.iter().any(|value| value
            .as_str()
            .is_some_and(|message| message.contains("parsing agent server config")))));
    assert!(diagnostics["managed_installs"]["error"]
        .as_str()
        .is_some_and(|value| value.contains("parsing agent server config")));
}

#[tokio::test]
async fn unauthenticated_health_omits_sensitive_fields_when_daemon_auth_is_enabled() {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        Some("daemon-secret".to_string()),
    ));
    let app = api::router(state);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/health")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let health: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        health.get("auth_required").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert!(health.get("compatibility").is_some());
    assert_eq!(
        health
            .pointer("/compatibility/desktop_dev_instance_id")
            .and_then(|v| v.as_str()),
        Some(""),
        "health leaked desktop_dev_instance_id: {health:#?}"
    );
    assert_eq!(
        health
            .pointer("/compatibility/protocol_compatibility_token")
            .and_then(|v| v.as_str()),
        Some(""),
        "health leaked protocol_compatibility_token: {health:#?}"
    );
    assert!(
        health.get("pid").is_none(),
        "health leaked pid: {health:#?}"
    );
    assert!(
        health.get("data_root").is_none(),
        "health leaked data_root: {health:#?}"
    );
    assert!(
        health.get("daemon_url").is_none(),
        "health leaked daemon_url: {health:#?}"
    );
    assert!(
        health.get("storage").is_none(),
        "health leaked storage state: {health:#?}"
    );
}

#[tokio::test]
async fn authorized_health_keeps_sensitive_fields_when_daemon_auth_is_enabled() {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        Some("daemon-secret".to_string()),
    ));
    let app = api::router(state);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/health")
        .header(header::AUTHORIZATION, "Bearer daemon-secret")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let health: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert!(health.get("pid").is_some(), "authorized health missing pid");
    assert!(
        health.get("data_root").and_then(|v| v.as_str()).is_some(),
        "authorized health missing data_root"
    );
    assert!(
        health.get("daemon_url").and_then(|v| v.as_str()).is_some(),
        "authorized health missing daemon_url"
    );
    assert!(
        health
            .pointer("/storage/level")
            .and_then(|v| v.as_str())
            .is_some(),
        "authorized health missing storage state"
    );
    assert_ne!(
        health
            .pointer("/compatibility/desktop_dev_instance_id")
            .and_then(|v| v.as_str()),
        Some(""),
        "authorized health missing desktop_dev_instance_id"
    );
    assert_ne!(
        health
            .pointer("/compatibility/protocol_compatibility_token")
            .and_then(|v| v.as_str()),
        Some(""),
        "authorized health missing protocol_compatibility_token"
    );
}
