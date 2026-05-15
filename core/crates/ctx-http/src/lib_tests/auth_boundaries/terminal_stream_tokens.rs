use super::*;

#[tokio::test]
async fn terminal_websocket_stream_requires_terminal_scoped_query_token() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());
    let git_repo = setup_git_repo().await;

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = test_daemon(data_dir.path(), stores, Some("daemon-secret".to_string()));
    let app = test_router(&state);

    let req = Request::builder()
        .method("POST")
        .uri("/api/workspaces")
        .header("authorization", "Bearer daemon-secret")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "root_path": git_repo.path().to_string_lossy(),
                "name": "terminal-auth-boundary"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let workspace: ctx_core::models::Workspace = serde_json::from_slice(&body).unwrap();

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/workspaces/{}/terminals", workspace.id.0))
        .header("authorization", "Bearer daemon-secret")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"cwd": git_repo.path().to_string_lossy()}).to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let terminal: ctx_core::models::TerminalSession = serde_json::from_slice(&body).unwrap();

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/terminals/{}/stream_token", terminal.id.0))
        .header("authorization", "Bearer daemon-secret")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let stream_path = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["stream_path"]
        .as_str()
        .unwrap()
        .to_string();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = reqwest::Client::new();

    assert_eq!(
        websocket_upgrade_status(
            &client,
            addr,
            &format!(
                "/api/terminals/{}/stream?token=daemon-secret",
                terminal.id.0
            ),
        )
        .await,
        StatusCode::UNAUTHORIZED
    );

    assert_eq!(
        websocket_upgrade_status(
            &client,
            addr,
            &format!("/api/terminals/{}/stream", terminal.id.0)
        )
        .await,
        StatusCode::UNAUTHORIZED
    );

    assert_eq!(
        websocket_upgrade_status(&client, addr, &stream_path).await,
        StatusCode::SWITCHING_PROTOCOLS
    );

    assert_eq!(
        websocket_upgrade_status(&client, addr, &stream_path).await,
        StatusCode::UNAUTHORIZED
    );

    server.abort();
}
