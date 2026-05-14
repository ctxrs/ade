use super::*;

#[tokio::test]
async fn provider_options_surface_agent_server_config_errors() {
    let _serial = home_env_test_lock().lock().await;
    let git_repo = setup_git_repo().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    write_invalid_agent_server_config(data_dir.path());
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = Arc::new(DaemonState::new(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let app = api::router(state);
    let workspace = create_workspace_via_api(&app, &git_repo.path().to_string_lossy()).await;

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/workspaces/{}/providers/qwen/options",
            workspace.id.0
        ))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["probe_ok"].as_bool(), Some(false));
    assert!(payload["config_error"]
        .as_str()
        .is_some_and(|value| value.contains("parsing agent server config")));
}

#[tokio::test]
async fn provider_options_ignore_stale_verify_cache_while_agent_server_config_is_broken() {
    let _serial = home_env_test_lock().lock().await;
    let git_repo = setup_git_repo().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = Arc::new(DaemonState::new(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let app = api::router(state);
    let workspace = create_workspace_via_api(&app, &git_repo.path().to_string_lossy()).await;

    let verify_req = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/workspaces/{}/providers/qwen/verify",
            workspace.id.0
        ))
        .body(Body::empty())
        .unwrap();
    let verify_res = app.clone().oneshot(verify_req).await.unwrap();
    assert_eq!(verify_res.status(), StatusCode::OK);

    write_invalid_agent_server_config(data_dir.path());

    let broken_options_req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/workspaces/{}/providers/qwen/options",
            workspace.id.0
        ))
        .body(Body::empty())
        .unwrap();
    let broken_options_res = app.clone().oneshot(broken_options_req).await.unwrap();
    assert_eq!(broken_options_res.status(), StatusCode::OK);
    let broken_options_body = to_bytes(broken_options_res.into_body(), usize::MAX)
        .await
        .unwrap();
    let broken_payload: serde_json::Value = serde_json::from_slice(&broken_options_body).unwrap();
    assert!(broken_payload["config_error"]
        .as_str()
        .is_some_and(|value| value.contains("parsing agent server config")));
    assert!(
        broken_payload.get("verify").is_none(),
        "stale verify cache should be ignored while config is broken: {broken_payload:#?}"
    );

    clear_agent_server_config(data_dir.path());

    let repaired_options_req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/workspaces/{}/providers/qwen/options",
            workspace.id.0
        ))
        .body(Body::empty())
        .unwrap();
    let repaired_options_res = app.clone().oneshot(repaired_options_req).await.unwrap();
    assert_eq!(repaired_options_res.status(), StatusCode::OK);
    let repaired_options_body = to_bytes(repaired_options_res.into_body(), usize::MAX)
        .await
        .unwrap();
    let repaired_payload: serde_json::Value =
        serde_json::from_slice(&repaired_options_body).unwrap();
    assert!(
        repaired_payload.get("config_error").is_none(),
        "broken config response should not poison later healthy options responses: {repaired_payload:#?}"
    );
}
