use super::*;

#[tokio::test]
async fn pair_mobile_device_rejects_disabled_mobile_access_even_with_valid_token() {
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
        None,
    ));
    let profile_id = insert_mobile_profile(&state).await;
    state
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
            enabled: false,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    let token = "valid-pairing-token";
    let mut hasher = sha2::Sha256::new();
    hasher.update(token.as_bytes());
    let token_hash = hex::encode(hasher.finalize());
    state
        .global_store()
        .insert_mobile_pairing_token(
            "pair-1",
            &token_hash,
            chrono::Utc::now() + chrono::Duration::minutes(5),
        )
        .await
        .unwrap();

    let app = api::router(state.clone());
    let req = Request::builder()
        .method("POST")
        .uri("/api/mobile/pair")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "pairing_token": token,
                "device_id": "33333333-3333-3333-3333-333333333333",
                "device_label": "phone",
                "platform": "ios",
                "public_key": "device-public"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["error"], "mobile access not enabled");
    assert!(
        state
            .global_store()
            .consume_mobile_pairing_token(&token_hash)
            .await
            .unwrap(),
        "disabled mobile access should not consume a valid pairing token"
    );
}

#[tokio::test]
async fn pair_mobile_device_preserves_token_for_invalid_device_id() {
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
        None,
    ));
    let profile_id = insert_mobile_profile(&state).await;
    state
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

    let token = "valid-pairing-token";
    let mut hasher = sha2::Sha256::new();
    hasher.update(token.as_bytes());
    let token_hash = hex::encode(hasher.finalize());
    state
        .global_store()
        .insert_mobile_pairing_token(
            "pair-1",
            &token_hash,
            chrono::Utc::now() + chrono::Duration::minutes(5),
        )
        .await
        .unwrap();

    let app = api::router(state.clone());
    let req = Request::builder()
        .method("POST")
        .uri("/api/mobile/pair")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "pairing_token": token,
                "device_id": "not-a-uuid",
                "device_label": "phone",
                "platform": "ios",
                "public_key": "device-public"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["error"], "device_id must be a UUID");
    assert!(
        state
            .global_store()
            .consume_mobile_pairing_token(&token_hash)
            .await
            .unwrap(),
        "invalid device ids should not consume a valid pairing token"
    );
}

#[tokio::test]
async fn disable_mobile_access_clears_outstanding_pairing_tokens() {
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
        None,
    ));
    let profile_id = insert_mobile_profile(&state).await;
    state
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
    state
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
    let mut hasher = sha2::Sha256::new();
    hasher.update(token.as_bytes());
    let token_hash = hex::encode(hasher.finalize());
    state
        .global_store()
        .insert_mobile_pairing_token(
            "pair-1",
            &token_hash,
            chrono::Utc::now() + chrono::Duration::minutes(5),
        )
        .await
        .unwrap();

    let app = api::router(state.clone());
    let req = Request::builder()
        .method("POST")
        .uri("/api/mobile/access/disable")
        .header("content-type", "application/json")
        .body(Body::from(json!({ "supabase_token": "token" }).to_string()))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NO_CONTENT);

    let cfg = state
        .global_store()
        .get_mobile_access_config()
        .await
        .unwrap();
    assert!(cfg.is_none());
    let profile = state
        .global_store()
        .get_mobile_connection_profile(profile_id)
        .await
        .unwrap();
    assert!(profile.is_none());
    let device = state
        .global_store()
        .get_mobile_device(MobileDeviceId(
            uuid::Uuid::parse_str("55555555-5555-5555-5555-555555555555").unwrap(),
        ))
        .await
        .unwrap();
    assert!(device.is_none());
    let still_allowed = state
        .global_store()
        .consume_mobile_pairing_token(&token_hash)
        .await
        .unwrap();
    assert!(!still_allowed);
}
