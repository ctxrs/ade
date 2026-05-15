use super::*;

#[tokio::test]
async fn disable_mobile_access_clears_outstanding_pairing_tokens() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let daemon = test_daemon(data_dir.path(), stores, None);
    let profile_id = insert_mobile_profile(&daemon).await;
    daemon
        .global_store()
        .upsert_mobile_access_config(MobileAccessConfig {
            id: "default".to_string(),
            profile_id,
            tunnel_id: "tunnel-1".to_string(),
            public_base_url: "https://example.com".to_string(),
            relay_base_url: "https://relay.example.com".to_string(),
            tunnel_secret: "secret".to_string(),
            daemon_public_key: "daemon-public".to_string(),
            daemon_private_key: "daemon-private".to_string(),
            enabled: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap();
    daemon
        .global_store()
        .upsert_mobile_device(
            MobileDeviceId(uuid::Uuid::parse_str("55555555-5555-5555-5555-555555555555").unwrap()),
            profile_id,
            MobileDeviceUpsert {
                device_label: Some("old-phone".to_string()),
                platform: Some("ios".to_string()),
                push_token: None,
                push_provider: None,
                public_key: Some("device-public".to_string()),
                app_version: Some("1.0.0".to_string()),
            },
        )
        .await
        .unwrap();

    let token = "pairing-token-to-clear";
    let token_hash = pairing_token_hash(token);
    daemon
        .global_store()
        .insert_mobile_pairing_token(
            "pair-1",
            &token_hash,
            chrono::Utc::now() + chrono::Duration::minutes(5),
        )
        .await
        .unwrap();

    let app = test_router(&daemon);
    let req = Request::builder()
        .method("POST")
        .uri("/api/mobile/access/disable")
        .header("content-type", "application/json")
        .body(Body::from(json!({ "supabase_token": "token" }).to_string()))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NO_CONTENT);

    let cfg = daemon
        .global_store()
        .get_mobile_access_config()
        .await
        .unwrap();
    assert!(cfg.is_none());
    let profile = daemon
        .global_store()
        .get_mobile_connection_profile(profile_id)
        .await
        .unwrap();
    assert!(profile.is_none());
    let device = daemon
        .global_store()
        .get_mobile_device(MobileDeviceId(
            uuid::Uuid::parse_str("55555555-5555-5555-5555-555555555555").unwrap(),
        ))
        .await
        .unwrap();
    assert!(device.is_none());
    let still_allowed = daemon
        .global_store()
        .consume_mobile_pairing_token(&token_hash)
        .await
        .unwrap();
    assert!(!still_allowed);
}
