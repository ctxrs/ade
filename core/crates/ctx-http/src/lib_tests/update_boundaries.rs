use super::*;

#[tokio::test]
async fn update_check_rejects_path_traversal_channel() {
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
        .uri("/api/updates/check?channel=x/../../../secret")
        .header(header::AUTHORIZATION, "Bearer daemon-secret")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}
