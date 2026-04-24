use super::*;
use sha2::Digest;

#[tokio::test]
async fn delete_mobile_connection_profile_returns_not_found_after_first_removal() {
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

    let app = api::router(state);
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/mobile/connection_profiles/{}", profile.id.0))
        .header(header::AUTHORIZATION, "Bearer daemon-secret")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NO_CONTENT);

    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/mobile/connection_profiles/{}", profile.id.0))
        .header(header::AUTHORIZATION, "Bearer daemon-secret")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}
