use super::*;

#[tokio::test]
async fn update_check_rejects_path_traversal_channel() {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = test_daemon(data_dir.path(), stores, Some("daemon-secret".to_string()));
    let app = test_router(&state);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/updates/check?channel=x/../../../secret")
        .header(header::AUTHORIZATION, "Bearer daemon-secret")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn daemon_shutdown_endpoint_terminalizes_running_turns_before_ack() {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = {
        let _serial = home_env_test_lock().lock().await;
        let _shutdown_token =
            EnvVarGuard::set("CTX_LOCAL_DAEMON_SHUTDOWN_TOKEN", "local-shutdown-secret");
        test_daemon(data_dir.path(), stores, Some("daemon-secret".to_string()))
    };

    let fixture = state
        .seed_shutdown_running_turn_for_test(&data_dir.path().join("ws"), "fake", "model")
        .await
        .unwrap();

    let app = test_router(&state);
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/daemon/shutdown")
        .header(header::AUTHORIZATION, "Bearer daemon-secret")
        .header("x-ctx-local-daemon-shutdown-token", "local-shutdown-secret")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({"confirm": true}).to_string()))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let status = state
        .session_turn_status_for_test(fixture.session_id, fixture.turn_id)
        .await
        .unwrap()
        .expect("turn exists");
    assert_eq!(status, ctx_core::models::SessionTurnStatus::Interrupted);
}

#[tokio::test]
async fn daemon_shutdown_endpoint_requires_local_shutdown_token() {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = {
        let _serial = home_env_test_lock().lock().await;
        let _shutdown_token =
            EnvVarGuard::set("CTX_LOCAL_DAEMON_SHUTDOWN_TOKEN", "local-shutdown-secret");
        test_daemon(data_dir.path(), stores, Some("daemon-secret".to_string()))
    };
    let app = test_router(&state);

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/daemon/shutdown")
        .header(header::AUTHORIZATION, "Bearer daemon-secret")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({"confirm": true}).to_string()))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}
