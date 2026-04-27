use super::*;
use sha2::Digest;

#[tokio::test]
async fn mobile_api_tokens_do_not_authorize_desktop_api_routes() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());
    let git_repo = setup_git_repo().await;

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        Some("daemon-secret".to_string()),
    ));

    let token = "ctxm_test_mobile_token";
    let mut hasher = sha2::Sha256::new();
    hasher.update(token.as_bytes());
    let token_hash = hex::encode(hasher.finalize());
    state
        .global_store()
        .create_mobile_connection_profile(
            "mobile".to_string(),
            "https://example.com".to_string(),
            token_hash,
            "ctxm_tes".to_string(),
            vec!["device_registration".to_string()],
        )
        .await
        .unwrap();

    let app = api::router(state);
    let req = Request::builder()
        .method("GET")
        .uri("/api/workspaces")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    let req = Request::builder()
        .method("POST")
        .uri("/api/workspaces")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "root_path": git_repo.path().to_string_lossy(),
                "name": "mobile-token-ws"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn mobile_api_tokens_still_authorize_mobile_registration() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        Some("daemon-secret".to_string()),
    ));

    let token = "ctxm_test_mobile_token";
    let mut hasher = sha2::Sha256::new();
    hasher.update(token.as_bytes());
    let token_hash = hex::encode(hasher.finalize());
    state
        .global_store()
        .create_mobile_connection_profile(
            "mobile".to_string(),
            "https://example.com".to_string(),
            token_hash,
            "ctxm_tes".to_string(),
            vec!["device_registration".to_string()],
        )
        .await
        .unwrap();

    let app = api::router(state);
    let req = Request::builder()
        .method("POST")
        .uri("/api/mobile/register")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "device_id": "11111111-1111-1111-1111-111111111111",
                "device_label": "test phone"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn mobile_api_tokens_without_device_registration_scope_reject_registration() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        Some("daemon-secret".to_string()),
    ));

    let token = "ctxm_test_mobile_token";
    let mut hasher = sha2::Sha256::new();
    hasher.update(token.as_bytes());
    let token_hash = hex::encode(hasher.finalize());
    state
        .global_store()
        .create_mobile_connection_profile(
            "mobile".to_string(),
            "https://example.com".to_string(),
            token_hash,
            "ctxm_tes".to_string(),
            vec!["workspace_read".to_string()],
        )
        .await
        .unwrap();

    let app = api::router(state);
    let req = Request::builder()
        .method("POST")
        .uri("/api/mobile/register")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "device_id": "11111111-1111-1111-1111-111111111111",
                "device_label": "test phone"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        payload["error"],
        "mobile profile lacks device_registration scope"
    );
}

#[tokio::test]
async fn legacy_empty_scope_mobile_tokens_migrate_to_default_scopes_on_registration() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        Some("daemon-secret".to_string()),
    ));

    let token = "ctxm_test_mobile_token";
    let mut hasher = sha2::Sha256::new();
    hasher.update(token.as_bytes());
    let token_hash = hex::encode(hasher.finalize());
    let profile = state
        .global_store()
        .create_mobile_connection_profile(
            "mobile".to_string(),
            "https://example.com".to_string(),
            token_hash,
            "ctxm_tes".to_string(),
            Vec::new(),
        )
        .await
        .unwrap();

    let app = api::router(state.clone());
    let req = Request::builder()
        .method("POST")
        .uri("/api/mobile/register")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "device_id": "12121212-1212-1212-1212-121212121212",
                "device_label": "test phone"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let profile = state
        .global_store()
        .get_mobile_connection_profile(profile.id)
        .await
        .unwrap()
        .expect("profile should still exist");
    assert_eq!(
        profile.scopes,
        vec![
            "device_registration".to_string(),
            "workspace_read".to_string(),
            "workspace_stream".to_string(),
        ]
    );
}
