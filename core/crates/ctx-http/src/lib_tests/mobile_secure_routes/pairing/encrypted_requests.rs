use super::*;

#[tokio::test]
async fn pair_mobile_device_accepts_encrypted_pairing_request() {
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
    let (daemon_public_key, daemon_private_key) =
        ctx_transport_runtime::mobile_e2ee::generate_keypair();
    state
        .global_store()
        .upsert_mobile_access_config(MobileAccessConfig {
            id: "default".to_string(),
            profile_id,
            tunnel_id: "tunnel-1".to_string(),
            public_base_url: "https://example.com".to_string(),
            relay_base_url: "https://relay.example.com".to_string(),
            tunnel_secret: "secret".to_string(),
            daemon_public_key: daemon_public_key.clone(),
            daemon_private_key,
            enabled: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    let token = "valid-pairing-token";
    let token_hash = pairing_token_hash(token);
    state
        .global_store()
        .insert_mobile_pairing_token(
            "pair-1",
            &token_hash,
            chrono::Utc::now() + chrono::Duration::minutes(5),
        )
        .await
        .unwrap();

    let device_id = "33333333-3333-3333-3333-333333333333";
    let (device_public_key, device_secret_key) =
        ctx_transport_runtime::mobile_e2ee::generate_keypair();
    let body = encrypted_pair_request_value(
        token,
        device_id,
        &daemon_public_key,
        &device_public_key,
        &device_secret_key,
    );
    let serialized = body.to_string();
    assert!(!serialized.contains(token));
    assert!(!serialized.contains("phone"));
    assert!(!serialized.contains("ios"));
    assert!(!serialized.contains("1.0.0"));

    let app = api::router(state.clone());
    let mut mixed_body = body.clone();
    let mixed_object = mixed_body.as_object_mut().unwrap();
    mixed_object.insert("pairing_token".to_string(), json!(token));
    mixed_object.insert("device_label".to_string(), json!("phone"));
    mixed_object.insert("platform".to_string(), json!("ios"));
    mixed_object.insert("app_version".to_string(), json!("1.0.0"));
    let req = Request::builder()
        .method("POST")
        .uri("/api/mobile/pair")
        .header("content-type", "application/json")
        .body(Body::from(mixed_body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        res.status(),
        StatusCode::BAD_REQUEST,
        "encrypted pairing must reject mixed relay-visible legacy plaintext fields"
    );

    let req = Request::builder()
        .method("POST")
        .uri("/api/mobile/pair")
        .header("content-type", "application/json")
        .body(Body::from(serialized))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let envelope: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let key = ctx_transport_runtime::mobile_e2ee::derive_client_key(
        device_id,
        &device_secret_key,
        &daemon_public_key,
    )
    .unwrap();
    let plaintext = ctx_transport_runtime::mobile_e2ee::decrypt(
        &key,
        device_id,
        envelope["seq"].as_i64().unwrap(),
        envelope["nonce"].as_str().unwrap(),
        envelope["ciphertext"].as_str().unwrap(),
    )
    .unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&plaintext).unwrap();
    assert_eq!(payload["paired"], true);

    let device = state
        .global_store()
        .get_mobile_device(MobileDeviceId(uuid::Uuid::parse_str(device_id).unwrap()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        device.public_key.as_deref(),
        Some(device_public_key.as_str())
    );
    assert!(!state
        .global_store()
        .consume_mobile_pairing_token(&token_hash)
        .await
        .unwrap());
}

#[tokio::test]
async fn pair_mobile_device_rejects_plaintext_legacy_pairing_request_without_consuming_token() {
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
    let (daemon_public_key, daemon_private_key) =
        ctx_transport_runtime::mobile_e2ee::generate_keypair();
    state
        .global_store()
        .upsert_mobile_access_config(MobileAccessConfig {
            id: "default".to_string(),
            profile_id,
            tunnel_id: "tunnel-1".to_string(),
            public_base_url: "https://example.com".to_string(),
            relay_base_url: "https://relay.example.com".to_string(),
            tunnel_secret: "secret".to_string(),
            daemon_public_key,
            daemon_private_key,
            enabled: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    let token = "valid-pairing-token";
    let token_hash = pairing_token_hash(token);
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
                "public_key": "device-public",
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    assert!(
        state
            .global_store()
            .consume_mobile_pairing_token(&token_hash)
            .await
            .unwrap(),
        "legacy plaintext pairing should not consume a valid pairing token"
    );
}

#[tokio::test]
async fn pair_mobile_device_preserves_token_for_malformed_encrypted_request() {
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
    let (daemon_public_key, daemon_private_key) =
        ctx_transport_runtime::mobile_e2ee::generate_keypair();
    state
        .global_store()
        .upsert_mobile_access_config(MobileAccessConfig {
            id: "default".to_string(),
            profile_id,
            tunnel_id: "tunnel-1".to_string(),
            public_base_url: "https://example.com".to_string(),
            relay_base_url: "https://relay.example.com".to_string(),
            tunnel_secret: "secret".to_string(),
            daemon_public_key,
            daemon_private_key,
            enabled: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    let token = "valid-pairing-token";
    let token_hash = pairing_token_hash(token);
    state
        .global_store()
        .insert_mobile_pairing_token(
            "pair-1",
            &token_hash,
            chrono::Utc::now() + chrono::Duration::minutes(5),
        )
        .await
        .unwrap();

    let (device_public_key, _device_secret_key) =
        ctx_transport_runtime::mobile_e2ee::generate_keypair();
    let app = api::router(state.clone());
    let req = Request::builder()
        .method("POST")
        .uri("/api/mobile/pair")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "device_id": "33333333-3333-3333-3333-333333333333",
                "public_key": device_public_key,
                "seq": 0,
                "nonce": "invalid",
                "ciphertext": "invalid",
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    assert!(
        state
            .global_store()
            .consume_mobile_pairing_token(&token_hash)
            .await
            .unwrap(),
        "malformed encrypted pairing should not consume a valid pairing token"
    );
}
