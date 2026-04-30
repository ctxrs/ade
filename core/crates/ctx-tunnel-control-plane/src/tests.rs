use super::*;
use std::sync::Mutex as StdMutex;

static ENV_LOCK: StdMutex<()> = StdMutex::new(());

#[test]
fn extract_bearer_trims_and_rejects_invalid_values() {
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(AUTHORIZATION, "Bearer token-123   ".parse().unwrap());
    assert_eq!(extract_bearer(&headers).as_deref(), Some("token-123"));

    headers.insert(AUTHORIZATION, "Basic nope".parse().unwrap());
    assert!(extract_bearer(&headers).is_none());
}

#[test]
fn normalize_public_base_url_appends_tunnel_path_once() {
    assert_eq!(
        normalize_public_base_url("https://public.example/", "tunnel-1"),
        "https://public.example/t/tunnel-1"
    );
    assert_eq!(
        normalize_public_base_url("https://public.example/t/existing", "tunnel-1"),
        "https://public.example/t/existing"
    );
}

#[test]
fn derive_tunnel_secret_is_deterministic_and_distinct() {
    let first = derive_tunnel_secret(b"master-secret", "tunnel-1").unwrap();
    let second = derive_tunnel_secret(b"master-secret", "tunnel-1").unwrap();
    let third = derive_tunnel_secret(b"master-secret", "tunnel-2").unwrap();
    assert_eq!(first, second);
    assert_ne!(first, third);
    assert!(!first.contains('='));
}

#[test]
fn parse_master_secret_trims_and_rejects_empty_values() {
    assert_eq!(
        parse_master_secret("  master-secret \n").unwrap(),
        b"master-secret".to_vec()
    );
    assert!(parse_master_secret("").is_err());
    assert!(parse_master_secret("   ").is_err());
}

#[test]
fn load_master_secret_requires_canonical_env() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let previous = std::env::var("CTX_TUNNEL_MASTER_SECRET").ok();
    std::env::remove_var("CTX_TUNNEL_MASTER_SECRET");
    assert!(load_master_secret().is_err());

    std::env::set_var("CTX_TUNNEL_MASTER_SECRET", "secret-1");
    assert_eq!(load_master_secret().unwrap(), b"secret-1");

    match previous {
        Some(value) => std::env::set_var("CTX_TUNNEL_MASTER_SECRET", value),
        None => std::env::remove_var("CTX_TUNNEL_MASTER_SECRET"),
    }
}

#[test]
fn store_errors_map_to_operational_api_statuses() {
    let no_relay = store_api_error(TunnelStoreError::NoHealthyRelay {
        region: "us".to_string(),
    });
    assert_eq!(no_relay.0, StatusCode::SERVICE_UNAVAILABLE);

    let db = store_api_error(TunnelStoreError::Database("down".to_string()));
    assert_eq!(db.0, StatusCode::INTERNAL_SERVER_ERROR);
}
