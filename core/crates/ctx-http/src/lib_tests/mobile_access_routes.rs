use super::*;

async fn spawn_mobile_enable_control_plane() -> (tokio::task::JoinHandle<()>, EnvVarGuard) {
    let control_plane = axum::Router::new().route(
        "/v1/mobile/enable",
        axum::routing::post(|| async move {
            axum::Json(json!({
                "tunnel_id": "tunnel-1",
                "public_base_url": "https://remote.example.com",
                "relay_base_url": "https://relay.example.com",
                "tunnel_secret": "secret"
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, control_plane).await.unwrap();
    });
    let control_plane_url =
        EnvVarGuard::set("CTX_TUNNEL_CONTROL_PLANE_URL", &format!("http://{addr}"));
    (server, control_plane_url)
}

#[tokio::test]
async fn enable_mobile_access_seeds_explicit_default_scopes() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let (server, _control_plane_url) = spawn_mobile_enable_control_plane().await;

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        Some("daemon-secret".to_string()),
    ));

    let app = api::router(state.clone());
    let req = Request::builder()
        .method("POST")
        .uri("/api/mobile/access/enable")
        .header(header::AUTHORIZATION, "Bearer daemon-secret")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "supabase_token": "entitled-token"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let cfg = state
        .global_store()
        .get_mobile_access_config()
        .await
        .unwrap()
        .expect("mobile access config should be stored");
    let profile = state
        .global_store()
        .get_mobile_connection_profile(cfg.profile_id)
        .await
        .unwrap()
        .expect("managed mobile profile should exist");
    assert_eq!(
        profile.scopes,
        vec![
            "device_registration".to_string(),
            "workspace_read".to_string(),
            "workspace_stream".to_string(),
        ]
    );

    state.transport.mobile_tunnel.stop().await;
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn enable_mobile_access_backfills_empty_managed_profile_scopes() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let (server, _control_plane_url) = spawn_mobile_enable_control_plane().await;

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        Some("daemon-secret".to_string()),
    ));

    let legacy_profile = state
        .global_store()
        .create_mobile_connection_profile(
            "Managed Mobile Access".to_string(),
            "https://legacy.example.com".to_string(),
            "legacy-token-hash".to_string(),
            "legacy-m".to_string(),
            Vec::new(),
        )
        .await
        .unwrap();
    let (daemon_public_key, daemon_private_key) =
        ctx_transport_runtime::mobile_e2ee::generate_keypair();
    state
        .global_store()
        .upsert_mobile_access_config(ctx_store::store::MobileAccessConfig {
            id: "default".to_string(),
            profile_id: legacy_profile.id,
            tunnel_id: "legacy-tunnel".to_string(),
            public_base_url: "https://legacy.example.com".to_string(),
            relay_base_url: "https://legacy-relay.example.com".to_string(),
            tunnel_secret: "legacy-secret".to_string(),
            daemon_public_key,
            daemon_private_key,
            enabled: false,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    let app = api::router(state.clone());
    let req = Request::builder()
        .method("POST")
        .uri("/api/mobile/access/enable")
        .header(header::AUTHORIZATION, "Bearer daemon-secret")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "supabase_token": "entitled-token"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let cfg = state
        .global_store()
        .get_mobile_access_config()
        .await
        .unwrap()
        .expect("mobile access config should be stored");
    assert_eq!(cfg.profile_id, legacy_profile.id);
    let profile = state
        .global_store()
        .get_mobile_connection_profile(cfg.profile_id)
        .await
        .unwrap()
        .expect("managed mobile profile should exist");
    assert_eq!(
        profile.scopes,
        vec![
            "device_registration".to_string(),
            "workspace_read".to_string(),
            "workspace_stream".to_string(),
        ]
    );

    state.transport.mobile_tunnel.stop().await;
    server.abort();
    let _ = server.await;
}
