use super::*;
use base64::Engine;
use ctx_core::ids::{ConnectionProfileId, MobileDeviceId, WorkspaceId};
use sha2::Digest;
use ctx_store::store::{MobileAccessConfig, MobileDeviceUpsert};

const TEST_MOBILE_API_TOKEN: &str = "ctxm_test_mobile_api_token";

async fn insert_mobile_profile(state: &Arc<AppState>) -> ConnectionProfileId {
    let mut hasher = sha2::Sha256::new();
    hasher.update(TEST_MOBILE_API_TOKEN.as_bytes());
    let token_hash = hex::encode(hasher.finalize());
    state
        .global_store()
        .create_mobile_connection_profile(
            "mobile".to_string(),
            "https://example.com".to_string(),
            token_hash,
            "ctxm_tes".to_string(),
            Vec::new(),
        )
        .await
        .unwrap()
        .id
}

async fn build_mobile_access_app(
    enabled: bool,
) -> (
    axum::Router,
    Arc<AppState>,
    WorkspaceId,
    String,
    ctx_transport_runtime::mobile_e2ee::E2eeKey,
) {
    let git_repo = setup_git_repo().await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let app = api::router(state.clone());
    let workspace = create_workspace_via_api(&app, &git_repo.path().to_string_lossy()).await;
    let profile_id = insert_mobile_profile(&state).await;
    let device_id = "22222222-2222-2222-2222-222222222222".to_string();
    let (daemon_public_key, daemon_private_key) =
        ctx_transport_runtime::mobile_e2ee::generate_keypair();
    let (device_public_key, device_secret_key) =
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
            enabled,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap();
    state
        .global_store()
        .upsert_mobile_device(
            MobileDeviceId(uuid::Uuid::parse_str(&device_id).unwrap()),
            profile_id,
            MobileDeviceUpsert {
                device_label: Some("phone".to_string()),
                platform: Some("ios".to_string()),
                push_token: None,
                push_provider: None,
                public_key: Some(device_public_key),
                app_version: Some("1.0.0".to_string()),
            },
        )
        .await
        .unwrap();

    let key = ctx_transport_runtime::mobile_e2ee::derive_client_key(
        &device_id,
        &device_secret_key,
        &daemon_public_key,
    )
    .unwrap();

    (app, state, workspace.id, device_id, key)
}

async fn build_mobile_secure_proxy_app(
    enabled: bool,
) -> (
    axum::Router,
    Arc<AppState>,
    String,
    ctx_transport_runtime::mobile_e2ee::E2eeKey,
) {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        Some("daemon-secret".to_string()),
    ));
    let profile_id = insert_mobile_profile(&state).await;

    let device_id = "44444444-4444-4444-4444-444444444444".to_string();
    let (daemon_public_key, daemon_private_key) =
        ctx_transport_runtime::mobile_e2ee::generate_keypair();
    let (device_public_key, device_secret_key) =
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
            enabled,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap();
    state
        .global_store()
        .upsert_mobile_device(
            MobileDeviceId(uuid::Uuid::parse_str(&device_id).unwrap()),
            profile_id,
            MobileDeviceUpsert {
                device_label: Some("phone".to_string()),
                platform: Some("ios".to_string()),
                push_token: None,
                push_provider: None,
                public_key: Some(device_public_key),
                app_version: Some("1.0.0".to_string()),
            },
        )
        .await
        .unwrap();

    let key = ctx_transport_runtime::mobile_e2ee::derive_client_key(
        &device_id,
        &device_secret_key,
        &daemon_public_key,
    )
    .unwrap();
    (api::router(state.clone()), state, device_id, key)
}

async fn post_mobile_secure_request(
    app: &axum::Router,
    device_id: &str,
    key: &ctx_transport_runtime::mobile_e2ee::E2eeKey,
    seq: i64,
    payload: serde_json::Value,
) -> axum::response::Response {
    let plaintext = serde_json::to_vec(&payload).unwrap();
    let envelope = ctx_transport_runtime::mobile_e2ee::encrypt(key, device_id, seq, &plaintext).unwrap();
    let req = Request::builder()
        .method("POST")
        .uri("/api/mobile/secure")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "device_id": envelope.device_id,
                "seq": envelope.seq,
                "nonce": envelope.nonce_b64,
                "ciphertext": envelope.ciphertext_b64
            })
            .to_string(),
        ))
        .unwrap();
    app.clone().oneshot(req).await.unwrap()
}

async fn decode_mobile_secure_response(
    res: axum::response::Response,
    device_id: &str,
    key: &ctx_transport_runtime::mobile_e2ee::E2eeKey,
) -> serde_json::Value {
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let plaintext = ctx_transport_runtime::mobile_e2ee::decrypt(
        key,
        device_id,
        payload["seq"].as_i64().unwrap(),
        payload["nonce"].as_str().unwrap(),
        payload["ciphertext"].as_str().unwrap(),
    )
    .unwrap();
    serde_json::from_slice(&plaintext).unwrap()
}

fn mobile_secure_stream_query(
    device_id: &str,
    key: &ctx_transport_runtime::mobile_e2ee::E2eeKey,
    workspace_id: WorkspaceId,
) -> String {
    let token =
        ctx_transport_runtime::mobile_e2ee::derive_stream_token(key, &workspace_id.0.to_string());
    format!("device_id={device_id}&token={token}")
}

#[tokio::test]
async fn mobile_secure_workspace_stream_returns_unauthorized_before_upgrade_for_missing_workspace_without_mobile_access()
{
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
        Some("daemon-secret".to_string()),
    ));
    let app = api::router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = reqwest::Client::new();

    let res = client
        .get(format!(
            "http://{addr}/api/mobile/secure/workspaces/11111111-1111-1111-1111-111111111111/stream?device_id=22222222-2222-2222-2222-222222222222&token=bad-token"
        ))
        .header("connection", "upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    server.abort();
}

#[tokio::test]
async fn mobile_secure_workspace_stream_returns_not_found_before_upgrade_for_authorized_missing_workspace()
{
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let (app, _state, _workspace_id, device_id, key) = build_mobile_access_app(true).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = reqwest::Client::new();

    let missing_workspace_id =
        WorkspaceId(uuid::Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap());
    let query = mobile_secure_stream_query(&device_id, &key, missing_workspace_id);
    let res = client
        .get(format!(
            "http://{addr}/api/mobile/secure/workspaces/{}/stream?{query}",
            missing_workspace_id.0
        ))
        .header("connection", "upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    server.abort();
}

#[tokio::test]
async fn mobile_secure_workspace_stream_returns_unauthorized_before_upgrade_without_mobile_access()
{
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let git_repo = setup_git_repo().await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let app = api::router(state);
    let workspace = create_workspace_via_api(&app, &git_repo.path().to_string_lossy()).await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = reqwest::Client::new();

    let res = client
        .get(format!(
            "http://{addr}/api/mobile/secure/workspaces/{}/stream?device_id=22222222-2222-2222-2222-222222222222&token=bad-token",
            workspace.id.0
        ))
        .header("connection", "upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    server.abort();
}

#[tokio::test]
async fn mobile_secure_workspace_stream_rejects_disabled_mobile_access_before_upgrade() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let (app, _state, workspace_id, device_id, key) = build_mobile_access_app(false).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = reqwest::Client::new();

    let query = mobile_secure_stream_query(&device_id, &key, workspace_id);
    let res = client
        .get(format!(
            "http://{addr}/api/mobile/secure/workspaces/{}/stream?{query}",
            workspace_id.0
        ))
        .header("connection", "upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    server.abort();
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

    let app = api::router(state);
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
}

#[tokio::test]
async fn mobile_secure_proxy_rejects_disabled_mobile_access_for_existing_device() {
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
        Some("daemon-secret".to_string()),
    ));
    let profile_id = insert_mobile_profile(&state).await;

    let device_id = "44444444-4444-4444-4444-444444444444";
    let (daemon_public_key, daemon_private_key) = ctx_transport_runtime::mobile_e2ee::generate_keypair();
    let (device_public_key, device_secret_key) = ctx_transport_runtime::mobile_e2ee::generate_keypair();
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
            daemon_private_key: daemon_private_key.clone(),
            enabled: false,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap();
    state
        .global_store()
        .upsert_mobile_device(
            MobileDeviceId(uuid::Uuid::parse_str(device_id).unwrap()),
            profile_id,
            MobileDeviceUpsert {
                device_label: Some("phone".to_string()),
                platform: Some("ios".to_string()),
                push_token: None,
                push_provider: None,
                public_key: Some(device_public_key.clone()),
                app_version: Some("1.0.0".to_string()),
            },
        )
        .await
        .unwrap();

    let key = ctx_transport_runtime::mobile_e2ee::derive_client_key(
        device_id,
        &device_secret_key,
        &state
            .global_store()
            .get_mobile_access_config()
            .await
            .unwrap()
            .unwrap()
            .daemon_public_key,
    )
    .unwrap();
    let plaintext = serde_json::to_vec(&json!({
        "method": "GET",
        "path": "/api/workspaces",
        "headers": []
    }))
    .unwrap();
    let envelope = ctx_transport_runtime::mobile_e2ee::encrypt(&key, device_id, 1, &plaintext).unwrap();

    let app = api::router(state);
    let req = Request::builder()
        .method("POST")
        .uri("/api/mobile/secure")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "device_id": envelope.device_id,
                "seq": envelope.seq,
                "nonce": envelope.nonce_b64,
                "ciphertext": envelope.ciphertext_b64
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["error"], "mobile access not enabled");
}

#[tokio::test]
async fn mobile_secure_proxy_grants_mobile_auth_for_proxied_api_routes() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let (app, _state, device_id, key) = build_mobile_secure_proxy_app(true).await;
    let res = post_mobile_secure_request(
        &app,
        &device_id,
        &key,
        1,
        json!({
            "method": "GET",
            "path": "/api/workspaces",
            "headers": []
        }),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);

    let payload = decode_mobile_secure_response(res, &device_id, &key).await;
    assert_eq!(payload["status"], 200);
    let body_bytes = base64::engine::general_purpose::STANDARD
        .decode(payload["body_b64"].as_str().unwrap())
        .unwrap();
    let body_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(body_json, json!([]));
}

#[tokio::test]
async fn mobile_secure_proxy_rejects_mobile_management_paths_after_trimming() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let (app, state, device_id, key) = build_mobile_secure_proxy_app(true).await;
    let target_device_id = "55555555-5555-5555-5555-555555555555";
    let pairing_token = "pairing-token-through-secure-proxy";
    let mut hasher = sha2::Sha256::new();
    hasher.update(pairing_token.as_bytes());
    let token_hash = hex::encode(hasher.finalize());
    state
        .global_store()
        .insert_mobile_pairing_token(
            "pair-smuggle",
            &token_hash,
            chrono::Utc::now() + chrono::Duration::minutes(5),
        )
        .await
        .unwrap();

    let res = post_mobile_secure_request(
        &app,
        &device_id,
        &key,
        1,
        json!({
            "method": "POST",
            "path": " /api/mobile/pair",
            "headers": [["content-type", "application/json"]],
            "body_b64": base64::engine::general_purpose::STANDARD.encode(
                json!({
                    "pairing_token": pairing_token,
                    "device_id": target_device_id,
                    "device_label": "smuggled-device",
                    "platform": "ios",
                    "public_key": "smuggled-public-key"
                })
                .to_string()
            )
        }),
    )
    .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        payload["error"],
        "secure proxy cannot target mobile management endpoints"
    );
    assert!(
        state
            .global_store()
            .get_mobile_device(MobileDeviceId(uuid::Uuid::parse_str(target_device_id).unwrap()))
            .await
            .unwrap()
            .is_none(),
        "mobile secure proxy unexpectedly registered a device through a trimmed management path"
    );
}

#[tokio::test]
async fn mobile_secure_proxy_rejects_stale_sequence_without_rolling_back_counter() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let (app, _state, device_id, key) = build_mobile_secure_proxy_app(true).await;

    let first = post_mobile_secure_request(
        &app,
        &device_id,
        &key,
        2,
        json!({
            "method": "GET",
            "path": "/api/health",
            "headers": []
        }),
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    let first_payload = decode_mobile_secure_response(first, &device_id, &key).await;
    assert_eq!(first_payload["status"], 200);

    let stale = post_mobile_secure_request(
        &app,
        &device_id,
        &key,
        1,
        json!({
            "method": "GET",
            "path": "/api/health",
            "headers": []
        }),
    )
    .await;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    let stale_body = to_bytes(stale.into_body(), usize::MAX).await.unwrap();
    let stale_payload: serde_json::Value = serde_json::from_slice(&stale_body).unwrap();
    assert_eq!(stale_payload["error"], "stale request sequence");

    let replay = post_mobile_secure_request(
        &app,
        &device_id,
        &key,
        2,
        json!({
            "method": "GET",
            "path": "/api/health",
            "headers": []
        }),
    )
    .await;
    assert_eq!(replay.status(), StatusCode::CONFLICT);
    let replay_body = to_bytes(replay.into_body(), usize::MAX).await.unwrap();
    let replay_payload: serde_json::Value = serde_json::from_slice(&replay_body).unwrap();
    assert_eq!(replay_payload["error"], "stale request sequence");
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
