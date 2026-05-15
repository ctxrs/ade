use super::*;

#[tokio::test]
async fn dictation_websocket_stream_requires_browser_scoped_query_token() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = test_daemon(data_dir.path(), stores, Some("daemon-secret".to_string()));
    let app = test_router(&state);

    let expires_at = chrono::Utc::now().timestamp() + STREAM_TOKEN_TTL_SECS;
    let scoped_token = derive_browser_stream_token(
        "daemon-secret",
        &BrowserStreamAuthScope::DictationLivekit,
        expires_at,
    );
    let (addr, server) = serve_test_app(app).await;
    let client = reqwest::Client::new();

    assert_eq!(
        websocket_upgrade_status(
            &client,
            addr,
            "/api/dictation/livekit/stream?token=daemon-secret",
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        websocket_upgrade_status(&client, addr, "/api/dictation/livekit/stream").await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        websocket_upgrade_status(
            &client,
            addr,
            &format!("/api/dictation/livekit/stream?expires_at={expires_at}&token={scoped_token}"),
        )
        .await,
        StatusCode::SWITCHING_PROTOCOLS
    );

    server.abort();
}
