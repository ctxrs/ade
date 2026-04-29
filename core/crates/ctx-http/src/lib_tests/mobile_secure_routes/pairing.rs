use super::*;

fn pairing_token_hash(token: &str) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

fn encrypted_pair_request_value(
    pairing_token: &str,
    device_id: &str,
    daemon_public_key: &str,
    device_public_key: &str,
    device_secret_key: &str,
) -> serde_json::Value {
    let key = ctx_transport_runtime::mobile_e2ee::derive_client_key(
        device_id,
        device_secret_key,
        daemon_public_key,
    )
    .unwrap();
    let payload = json!({
        "pairing_token": pairing_token,
        "device_label": "phone",
        "platform": "ios",
        "app_version": "1.0.0",
    });
    let plaintext = serde_json::to_vec(&payload).unwrap();
    let envelope = ctx_transport_runtime::mobile_e2ee::encrypt_pairing_request(
        &key,
        device_id,
        device_public_key,
        &plaintext,
    )
    .unwrap();
    json!({
        "device_id": envelope.device_id,
        "public_key": device_public_key,
        "seq": envelope.seq,
        "nonce": envelope.nonce_b64,
        "ciphertext": envelope.ciphertext_b64,
    })
}

#[tokio::test]
async fn pair_mobile_device_rejects_profiles_without_device_registration_scope() {
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
    let profile_id =
        insert_mobile_profile_with_scopes(&state, &["workspace_read", "workspace_stream"]).await;
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
                "device_id": "33333333-3333-3333-3333-333333333333",
                "device_label": "phone",
                "platform": "ios",
                "public_key": "device-public"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        payload["error"],
        "mobile profile lacks device_registration scope"
    );
    assert!(
        state
            .global_store()
            .consume_mobile_pairing_token(&token_hash)
            .await
            .unwrap(),
        "scope failures should not consume a valid pairing token"
    );
}

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
                "device_id": "33333333-3333-3333-3333-333333333333",
                "public_key": "device-public",
                "seq": 0,
                "nonce": "invalid",
                "ciphertext": "invalid"
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
                "device_id": "not-a-uuid",
                "public_key": "device-public",
                "seq": 0,
                "nonce": "invalid",
                "ciphertext": "invalid"
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
