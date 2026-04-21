use super::*;

#[tokio::test]
async fn execution_launch_start_and_status_host_mode() {
    let _serial = home_env_test_lock().lock().await;
    let git_repo = setup_git_repo().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let app = api::router(state.clone());

    let req = Request::builder()
        .method("POST")
        .uri("/api/workspaces")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "root_path": git_repo.path().to_string_lossy(),
                "name": "ws"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let ws: ctx_core::models::Workspace = serde_json::from_slice(&body).unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/api/execution/launch/start")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "workspace_id": ws.id.0.to_string(),
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let snapshot: ExecutionLaunchSnapshot = serde_json::from_slice(&body).unwrap();
    assert_eq!(snapshot.workspace_id, ws.id.0.to_string());
    assert!(matches!(
        snapshot.state,
        ExecutionLaunchState::Running | ExecutionLaunchState::Ready
    ));
    assert!(!snapshot.job_id.trim().is_empty());

    let mut status_snapshot = snapshot.clone();
    for _ in 0..20 {
        let req = Request::builder()
            .method("GET")
            .uri(format!(
                "/api/execution/launch/status?job_id={}",
                snapshot.job_id
            ))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        status_snapshot = serde_json::from_slice(&body).unwrap();
        if status_snapshot.state == ExecutionLaunchState::Ready {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(status_snapshot.job_id, snapshot.job_id);
    assert_eq!(status_snapshot.state, ExecutionLaunchState::Ready);
}

#[tokio::test]
async fn execution_launch_startup_prewarm_kind_supported() {
    let _serial = sandbox_cli_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let sandbox_cli_path = write_failing_sandbox_cli_shim(data_dir.path());
    let _sandbox_cli = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let app = api::router(state.clone());

    let req = Request::builder()
        .method("POST")
        .uri("/api/execution/launch/start")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "kind": "startup_prewarm",
                "prewarm_scope": "builder",
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let snapshot: ExecutionLaunchSnapshot = serde_json::from_slice(&body).unwrap();
    assert_eq!(snapshot.kind, ExecutionSetupJobKind::StartupPrewarm);
    assert!(!snapshot.job_id.trim().is_empty());
    assert!(matches!(
        snapshot.state,
        ExecutionLaunchState::Running | ExecutionLaunchState::Error
    ));

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/execution/launch/status?job_id={}",
            snapshot.job_id
        ))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let mut status_snapshot: ExecutionLaunchSnapshot = serde_json::from_slice(&body).unwrap();
    assert_eq!(status_snapshot.job_id, snapshot.job_id);
    assert_eq!(status_snapshot.kind, ExecutionSetupJobKind::StartupPrewarm);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while matches!(status_snapshot.state, ExecutionLaunchState::Running)
        && tokio::time::Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(25)).await;
        let req = Request::builder()
            .method("GET")
            .uri(format!(
                "/api/execution/launch/status?job_id={}",
                snapshot.job_id
            ))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        status_snapshot = serde_json::from_slice(&body).unwrap();
    }
    assert!(matches!(
        status_snapshot.state,
        ExecutionLaunchState::Running | ExecutionLaunchState::Ready | ExecutionLaunchState::Error
    ));
}

#[tokio::test]
async fn execution_launch_start_returns_bad_request_when_execution_settings_fail() {
    let _serial = home_env_test_lock().lock().await;
    let git_repo = setup_git_repo().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let app = api::router(state.clone());

    let workspace = create_workspace_via_api(&app, &git_repo.path().to_string_lossy()).await;
    let store = state.store_for_workspace(workspace.id).await.unwrap();
    store
        .upsert_runtime_settings_document(1, "{")
        .await
        .unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/api/execution/launch/start")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "workspace_id": workspace.id.0.to_string(),
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(value["error"]
        .as_str()
        .unwrap_or_default()
        .contains("workspace runtime settings"));
}

#[tokio::test]
async fn ensure_workspace_container_returns_bad_request_when_execution_settings_fail() {
    let _serial = home_env_test_lock().lock().await;
    let git_repo = setup_git_repo().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let app = api::router(state.clone());

    let workspace = create_workspace_via_api(&app, &git_repo.path().to_string_lossy()).await;
    let store = state.store_for_workspace(workspace.id).await.unwrap();
    store
        .upsert_runtime_settings_document(1, "{")
        .await
        .unwrap();

    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/workspaces/{}/harness_container/ensure",
            workspace.id.0
        ))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(value["error"]
        .as_str()
        .unwrap_or_default()
        .contains("workspace runtime settings"));
}
