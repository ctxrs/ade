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
    let fixture =
        test_daemon_fixture_for_test(data_dir.path(), Some("daemon-secret".to_string())).await;
    let state = fixture.daemon();

    let app = fixture.router();
    let req = Request::builder()
        .method("POST")
        .uri("/api/mobile/access/enable")
        .header(header::AUTHORIZATION, "Bearer daemon-secret")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "managed_tunnel_grant": "ctmt_entitled_grant"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let cfg = state
        .mobile_access_for_test()
        .mobile_access_config_for_test()
        .await
        .unwrap()
        .expect("mobile access config should be stored");
    let profile = state
        .mobile_access_for_test()
        .mobile_profile_for_test(cfg.profile_id)
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

    state.stop_mobile_tunnel().await;
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
    let fixture =
        test_daemon_fixture_for_test(data_dir.path(), Some("daemon-secret".to_string())).await;
    let state = fixture.daemon();

    let legacy_profile = state
        .mobile_access_for_test()
        .seed_empty_managed_mobile_access_profile_for_test()
        .await
        .unwrap();
    let (daemon_public_key, daemon_private_key) =
        ctx_transport_runtime::mobile_e2ee::generate_keypair();
    state
        .mobile_access_for_test()
        .seed_legacy_mobile_access_config_for_test(
            legacy_profile.id,
            false,
            daemon_public_key,
            daemon_private_key,
        )
        .await
        .unwrap();

    let app = fixture.router();
    let req = Request::builder()
        .method("POST")
        .uri("/api/mobile/access/enable")
        .header(header::AUTHORIZATION, "Bearer daemon-secret")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "managed_tunnel_grant": "ctmt_entitled_grant"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let cfg = state
        .mobile_access_for_test()
        .mobile_access_config_for_test()
        .await
        .unwrap()
        .expect("mobile access config should be stored");
    assert_eq!(cfg.profile_id, legacy_profile.id);
    let profile = state
        .mobile_access_for_test()
        .mobile_profile_for_test(cfg.profile_id)
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

    state.stop_mobile_tunnel().await;
    server.abort();
    let _ = server.await;
}
