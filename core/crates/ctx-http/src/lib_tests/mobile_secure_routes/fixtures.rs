use super::*;
use ctx_core::ids::{ConnectionProfileId, MobileDeviceId, WorkspaceId};
use ctx_store::store::{MobileAccessConfig, MobileDeviceUpsert};
use sha2::Digest;

const TEST_MOBILE_API_TOKEN: &str = "ctxm_test_mobile_api_token";
const TEST_MOBILE_DEFAULT_SCOPES: &[&str] =
    &["device_registration", "workspace_read", "workspace_stream"];

pub(super) async fn insert_mobile_profile(state: &Arc<DaemonState>) -> ConnectionProfileId {
    insert_mobile_profile_with_scopes(state, TEST_MOBILE_DEFAULT_SCOPES).await
}

pub(super) async fn insert_mobile_profile_with_scopes(
    state: &Arc<DaemonState>,
    scopes: &[&str],
) -> ConnectionProfileId {
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
            scopes.iter().map(|scope| (*scope).to_string()).collect(),
        )
        .await
        .unwrap()
        .id
}

pub(super) async fn build_mobile_access_app(
    enabled: bool,
) -> (
    axum::Router,
    Arc<DaemonState>,
    WorkspaceId,
    String,
    ctx_transport_runtime::mobile_e2ee::E2eeKey,
    tempfile::TempDir,
) {
    build_mobile_access_app_with_scopes(enabled, TEST_MOBILE_DEFAULT_SCOPES).await
}

pub(super) async fn build_mobile_access_app_with_scopes(
    enabled: bool,
    scopes: &[&str],
) -> (
    axum::Router,
    Arc<DaemonState>,
    WorkspaceId,
    String,
    ctx_transport_runtime::mobile_e2ee::E2eeKey,
    tempfile::TempDir,
) {
    let git_repo = setup_git_repo().await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = Arc::new(DaemonState::new(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let app = api::router(state.clone());
    let workspace = create_workspace_via_api(&app, &git_repo.path().to_string_lossy()).await;
    let profile_id = insert_mobile_profile_with_scopes(&state, scopes).await;
    let device_id = "22222222-2222-2222-2222-222222222222".to_string();
    let (daemon_public_key, daemon_private_key) =
        ctx_transport_runtime::mobile_e2ee::generate_keypair();
    let (device_public_key, device_secret_key) =
        ctx_transport_runtime::mobile_e2ee::generate_keypair();

    seed_mobile_access_config(
        &state,
        profile_id,
        &device_id,
        daemon_public_key.clone(),
        daemon_private_key,
        device_public_key,
        enabled,
    )
    .await;

    let key = ctx_transport_runtime::mobile_e2ee::derive_client_key(
        &device_id,
        &device_secret_key,
        &daemon_public_key,
    )
    .unwrap();

    (app, state, workspace.id, device_id, key, data_dir)
}

pub(super) async fn build_mobile_secure_proxy_app(
    enabled: bool,
) -> (
    axum::Router,
    Arc<DaemonState>,
    String,
    ctx_transport_runtime::mobile_e2ee::E2eeKey,
    tempfile::TempDir,
) {
    build_mobile_secure_proxy_app_with_scopes(enabled, TEST_MOBILE_DEFAULT_SCOPES).await
}

pub(super) async fn build_mobile_secure_proxy_app_with_scopes(
    enabled: bool,
    scopes: &[&str],
) -> (
    axum::Router,
    Arc<DaemonState>,
    String,
    ctx_transport_runtime::mobile_e2ee::E2eeKey,
    tempfile::TempDir,
) {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = Arc::new(DaemonState::new(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        Some("daemon-secret".to_string()),
    ));
    let profile_id = insert_mobile_profile_with_scopes(&state, scopes).await;

    let device_id = "44444444-4444-4444-4444-444444444444".to_string();
    let (daemon_public_key, daemon_private_key) =
        ctx_transport_runtime::mobile_e2ee::generate_keypair();
    let (device_public_key, device_secret_key) =
        ctx_transport_runtime::mobile_e2ee::generate_keypair();
    seed_mobile_access_config(
        &state,
        profile_id,
        &device_id,
        daemon_public_key.clone(),
        daemon_private_key,
        device_public_key,
        enabled,
    )
    .await;

    let key = ctx_transport_runtime::mobile_e2ee::derive_client_key(
        &device_id,
        &device_secret_key,
        &daemon_public_key,
    )
    .unwrap();
    (api::router(state.clone()), state, device_id, key, data_dir)
}

async fn seed_mobile_access_config(
    state: &Arc<DaemonState>,
    profile_id: ConnectionProfileId,
    device_id: &str,
    daemon_public_key: String,
    daemon_private_key: String,
    device_public_key: String,
    enabled: bool,
) {
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
            enabled,
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
                public_key: Some(device_public_key),
                app_version: Some("1.0.0".to_string()),
            },
        )
        .await
        .unwrap();
}
