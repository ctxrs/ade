use super::*;
use ctx_core::ids::{ConnectionProfileId, MobileDeviceId, WorkspaceId};
use ctx_store::store::{MobileAccessConfig, MobileDeviceUpsert};
use sha2::Digest;

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
    let envelope =
        ctx_transport_runtime::mobile_e2ee::encrypt(key, device_id, seq, &plaintext).unwrap();
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

mod pairing;
mod proxy;
mod workspace_stream;
