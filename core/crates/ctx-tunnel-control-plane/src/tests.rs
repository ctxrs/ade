use super::*;
use axum::routing::get;
use axum::Router;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Mutex as StdMutex;
use tokio::sync::oneshot;

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
fn assign_relay_is_stable_for_same_user() {
    let relays = vec![
        "https://relay-a.example".to_string(),
        "https://relay-b.example".to_string(),
        "https://relay-c.example".to_string(),
    ];
    let assigned = assign_relay(&relays, "user-1");
    assert!(relays.contains(&assigned));
    assert_eq!(assign_relay(&relays, "user-1"), assigned);
    assert!(relays.contains(&assign_relay(&relays, "user-2")));
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

#[tokio::test]
async fn health_reports_ok_without_auth() {
    let resp = health().await.into_response();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[derive(Clone)]
struct MockDepsState {
    expected_token: String,
    user_id: String,
    entitled: bool,
}

async fn mock_auth_user(
    State(state): State<MockDepsState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let Some(token) = extract_bearer(&headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if token != state.expected_token {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({ "id": state.user_id })),
    )
        .into_response()
}

async fn mock_entitlements(State(state): State<MockDepsState>) -> impl IntoResponse {
    let remote_mobile_access = if state.entitled {
        "enabled"
    } else {
        "disabled"
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "plan_type": "pro",
            "features": { "remote_mobile_access": remote_mobile_access },
        })),
    )
        .into_response()
}

struct MockServer {
    base_url: String,
    shutdown: oneshot::Sender<()>,
}

async fn spawn_mock_deps_server(entitled: bool) -> MockServer {
    let state = MockDepsState {
        expected_token: "good-token".to_string(),
        user_id: "user-123".to_string(),
        entitled,
    };
    let app = Router::new()
        .route("/auth/v1/user", get(mock_auth_user))
        .route("/functions/v1/entitlements", get(mock_entitlements))
        .with_state(state);

    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind mock server");
    let local_addr = listener.local_addr().expect("local addr");
    let base_url = format!("http://{local_addr}");
    let (tx, rx) = oneshot::channel::<()>();

    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = rx.await;
            })
            .await;
    });

    MockServer {
        base_url,
        shutdown: tx,
    }
}

async fn setup_state(entitled: bool) -> (AppState, oneshot::Sender<()>) {
    let mock = spawn_mock_deps_server(entitled).await;

    let db = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");
    MIGRATOR.run(&db).await.expect("run migrations");

    let state = AppState {
        db,
        client: reqwest::Client::new(),
        config: ControlPlaneConfig {
            supabase_url: mock.base_url.clone(),
            supabase_anon_key: "anon-key".to_string(),
            entitlements_url: format!("{}/functions/v1/entitlements", mock.base_url),
            master_secret: b"master-secret".to_vec(),
            public_base_url: "https://public.example".to_string(),
            relay_base_urls: vec!["https://relay.example".to_string()],
        },
    };
    (state, mock.shutdown)
}

async fn count_tunnels(db: &Pool<Sqlite>) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM mobile_tunnels")
        .fetch_one(db)
        .await
        .expect("count tunnels")
}

async fn count_active_tunnels(db: &Pool<Sqlite>, user_id: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM mobile_tunnels WHERE user_id = ? AND disabled_at IS NULL",
    )
    .bind(user_id)
    .fetch_one(db)
    .await
    .expect("count active tunnels")
}

fn bearer_headers(token: &str) -> axum::http::HeaderMap {
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
    headers
}

#[tokio::test]
async fn enable_requires_authorization_and_does_not_insert_row() {
    let (state, shutdown) = setup_state(true).await;

    let headers = axum::http::HeaderMap::new();
    let err = enable_mobile_access_inner(state.clone(), headers)
        .await
        .expect_err("missing authorization should fail");
    assert_eq!(err.0, StatusCode::UNAUTHORIZED);
    assert_eq!(count_tunnels(&state.db).await, 0);

    let _ = shutdown.send(());
}

#[tokio::test]
async fn enable_denies_when_not_entitled_and_does_not_insert_row() {
    let (state, shutdown) = setup_state(false).await;

    let err = enable_mobile_access_inner(state.clone(), bearer_headers("good-token"))
        .await
        .expect_err("not entitled should fail");
    assert_eq!(err.0, StatusCode::FORBIDDEN);
    assert_eq!(count_tunnels(&state.db).await, 0);

    let _ = shutdown.send(());
}

#[tokio::test]
async fn enable_is_idempotent_for_active_tunnel() {
    let (state, shutdown) = setup_state(true).await;

    let first = enable_mobile_access_inner(state.clone(), bearer_headers("good-token"))
        .await
        .expect("first enable");
    assert_eq!(count_tunnels(&state.db).await, 1);

    let second = enable_mobile_access_inner(state.clone(), bearer_headers("good-token"))
        .await
        .expect("second enable");
    assert_eq!(count_tunnels(&state.db).await, 1);
    assert_eq!(first.tunnel_id, second.tunnel_id);
    assert_eq!(first.tunnel_secret, second.tunnel_secret);

    let _ = shutdown.send(());
}

#[tokio::test]
async fn revoke_disables_active_tunnel_and_next_enable_creates_new_one() {
    let (state, shutdown) = setup_state(true).await;

    let first = enable_mobile_access_inner(state.clone(), bearer_headers("good-token"))
        .await
        .expect("first enable");
    assert_eq!(count_active_tunnels(&state.db, "user-123").await, 1);

    let revoke_resp = revoke_mobile_access(State(state.clone()), bearer_headers("good-token"))
        .await
        .into_response();
    assert_eq!(revoke_resp.status(), StatusCode::NO_CONTENT);
    assert_eq!(count_active_tunnels(&state.db, "user-123").await, 0);

    let second = enable_mobile_access_inner(state.clone(), bearer_headers("good-token"))
        .await
        .expect("second enable after revoke");
    assert_ne!(first.tunnel_id, second.tunnel_id);
    assert_eq!(count_tunnels(&state.db).await, 2);
    assert_eq!(count_active_tunnels(&state.db, "user-123").await, 1);

    let _ = shutdown.send(());
}
