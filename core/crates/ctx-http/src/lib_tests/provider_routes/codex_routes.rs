use super::*;

#[tokio::test]
async fn codex_accounts_usage_surfaces_agent_server_config_errors() {
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
    let app = api::router(state);

    let req = Request::builder()
        .method("GET")
        .uri("/api/providers/codex/accounts/usage")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(payload["error"]
        .as_str()
        .is_some_and(|value| value.contains("parsing agent server config")));
}

#[tokio::test]
async fn provider_usage_cache_hit_surfaces_agent_server_config_errors_for_codex() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());
    let codex_home = tempfile::tempdir().unwrap();
    let _codex_home = EnvVarGuard::set("CTX_CODEX_HOME", &codex_home.path().to_string_lossy());

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
    state.providers.usage_cache.lock().await.insert(
        "codex-crp".to_string(),
        crate::provider_usage::ProviderUsageSnapshot {
            provider_id: "codex-crp".to_string(),
            source: "oauth".to_string(),
            fetched_at: chrono::Utc::now(),
            payload: Some(serde_json::json!({
                "cached": true
            })),
            error: None,
        },
    );
    let app = api::router(state);

    let req = Request::builder()
        .method("GET")
        .uri("/api/providers/codex/usage")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(payload["error"]
        .as_str()
        .is_some_and(|value| value.contains("parsing agent server config")));
}

#[tokio::test]
async fn provider_usage_cache_hit_projects_requested_provider_alias_for_codex() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());
    let codex_home = tempfile::tempdir().unwrap();
    let _codex_home = EnvVarGuard::set("CTX_CODEX_HOME", &codex_home.path().to_string_lossy());
    let codex_bin_dir = tempfile::tempdir().unwrap();
    let codex_bin = codex_bin_dir.path().join("codex-crp");
    std::fs::write(&codex_bin, "#!/bin/sh\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(&codex_bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let data_dir = tempfile::tempdir().unwrap();
    clear_agent_server_config(data_dir.path());
    let mut cfg = ctx_managed_installs::AgentServerConfigFile::default();
    cfg.providers.insert(
        "codex-cli".to_string(),
        ctx_managed_installs::AgentServerCommand {
            command: codex_bin.to_string_lossy().to_string(),
            args: Vec::new(),
            dependencies: Vec::new(),
            managed: None,
        },
    );
    ctx_managed_installs::save_agent_server_config(data_dir.path(), &cfg)
        .await
        .unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    state.providers.usage_cache.lock().await.insert(
        "codex-crp".to_string(),
        crate::provider_usage::ProviderUsageSnapshot {
            provider_id: "codex-crp".to_string(),
            source: "oauth".to_string(),
            fetched_at: chrono::Utc::now(),
            payload: Some(serde_json::json!({
                "cached": true
            })),
            error: None,
        },
    );
    let app = api::router(state);

    let req = Request::builder()
        .method("GET")
        .uri("/api/providers/codex/usage")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["provider_id"].as_str(), Some("codex"));
    assert_eq!(payload["payload"]["cached"].as_bool(), Some(true));
}

#[tokio::test]
async fn codex_login_start_surfaces_agent_server_config_errors() {
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
    let app = api::router(state);

    let req = Request::builder()
        .method("POST")
        .uri("/api/providers/codex/accounts/login/start")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"label":"test"}"#))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(payload["error"]
        .as_str()
        .is_some_and(|value| value.contains("parsing agent server config")));
}
