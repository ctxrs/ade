use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use base64::Engine;
use clap::Parser;
use hmac::{Hmac, Mac};
use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use sqlx::{sqlite::SqlitePoolOptions, Pool, Row, Sqlite};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::net::SocketAddr;
use tracing::{info, warn};
use url::Url;
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(name = "ctx-tunnel-control-plane", version)]
struct Args {
    #[arg(long, default_value = "0.0.0.0:8791")]
    listen: String,
}

#[derive(Clone)]
struct AppState {
    db: Pool<Sqlite>,
    client: reqwest::Client,
    config: ControlPlaneConfig,
    redis: Option<redis::aio::ConnectionManager>,
}

#[derive(Clone)]
struct ControlPlaneConfig {
    supabase_url: String,
    supabase_anon_key: String,
    entitlements_url: String,
    master_secret: Vec<u8>,
    public_base_url: String,
    relay_base_urls: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ApiErrorResp {
    error: String,
}

#[derive(Debug, Serialize)]
struct EnableMobileAccessResp {
    tunnel_id: String,
    public_base_url: String,
    relay_base_url: String,
    tunnel_secret: String,
}

#[derive(Debug, Deserialize)]
struct EntitlementsSnapshot {
    #[allow(dead_code)]
    plan_type: String,
    features: serde_json::Value,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let database_url = std::env::var("CONTROL_PLANE_DATABASE_URL")
        .context("missing CONTROL_PLANE_DATABASE_URL")?;
    let supabase_url = std::env::var("SUPABASE_URL").context("missing SUPABASE_URL")?;
    let supabase_anon_key =
        std::env::var("SUPABASE_ANON_KEY").context("missing SUPABASE_ANON_KEY")?;
    let entitlements_url = std::env::var("CONTROL_PLANE_ENTITLEMENTS_URL").unwrap_or_else(|_| {
        format!(
            "{}/functions/v1/entitlements",
            supabase_url.trim_end_matches('/')
        )
    });
    let public_base_url = std::env::var("MOBILE_TUNNEL_PUBLIC_BASE_URL")
        .context("missing MOBILE_TUNNEL_PUBLIC_BASE_URL")?;
    let relay_base_urls = std::env::var("MOBILE_TUNNEL_RELAY_BASE_URLS")
        .context("missing MOBILE_TUNNEL_RELAY_BASE_URLS")?
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    if relay_base_urls.is_empty() {
        anyhow::bail!("MOBILE_TUNNEL_RELAY_BASE_URLS must include at least one URL");
    }
    let master_secret = std::env::var("MOBILE_TUNNEL_MASTER_SECRET")
        .context("missing MOBILE_TUNNEL_MASTER_SECRET")?
        .into_bytes();

    let db = SqlitePoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await
        .context("connecting to database")?;
    let migrator = sqlx::migrate::Migrator::new(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations"),
    )
    .await
    .context("loading migrations")?;
    migrator.run(&db).await.context("running migrations")?;

    let redis = match std::env::var("CONTROL_PLANE_REDIS_URL") {
        Ok(url) if !url.trim().is_empty() => {
            let client = redis::Client::open(url)?;
            Some(redis::aio::ConnectionManager::new(client).await?)
        }
        _ => None,
    };

    let state = AppState {
        db,
        client: reqwest::Client::new(),
        config: ControlPlaneConfig {
            supabase_url,
            supabase_anon_key,
            entitlements_url,
            master_secret,
            public_base_url,
            relay_base_urls,
        },
        redis,
    };

    let app = Router::new()
        .route("/v1/mobile/enable", post(enable_mobile_access))
        .route("/v1/mobile/revoke", post(revoke_mobile_access))
        .with_state(state);

    let addr: SocketAddr = args.listen.parse().context("parsing --listen")?;
    info!("control plane listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("binding listener")?;
    axum::serve(listener, app)
        .await
        .context("serving control plane")?;
    Ok(())
}

async fn enable_mobile_access(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Response {
    match enable_mobile_access_inner(state, headers).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err((status, msg)) => (status, Json(ApiErrorResp { error: msg })).into_response(),
    }
}

async fn enable_mobile_access_inner(
    state: AppState,
    headers: axum::http::HeaderMap,
) -> Result<EnableMobileAccessResp, (StatusCode, String)> {
    let token = extract_bearer(&headers).ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            "authorization required".to_string(),
        )
    })?;
    tracing::info!("enable_mobile_access token_len={}", token.len());
    let user_id = fetch_user_id(&state, &token).await?;
    ensure_entitled(&state, &token).await?;

    if let Some(existing) = load_existing_tunnel(&state, &user_id).await? {
        return Ok(existing);
    }

    let tunnel_id = Uuid::new_v4().to_string();
    let relay_base_url = assign_relay(&state.config.relay_base_urls, &user_id);
    let public_base_url = normalize_public_base_url(&state.config.public_base_url, &tunnel_id);

    sqlx::query(
        r#"INSERT INTO mobile_tunnels
           (tunnel_id, user_id, relay_base_url, public_base_url, created_at)
           VALUES (?, ?, ?, ?, CURRENT_TIMESTAMP)"#,
    )
    .bind(&tunnel_id)
    .bind(&user_id)
    .bind(&relay_base_url)
    .bind(&public_base_url)
    .execute(&state.db)
    .await
    .map_err(|e| {
        warn!("failed to insert tunnel: {e}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to create tunnel".to_string(),
        )
    })?;

    let tunnel_secret = derive_tunnel_secret(&state.config.master_secret, &tunnel_id)?;
    let resp = EnableMobileAccessResp {
        tunnel_id: tunnel_id.clone(),
        public_base_url,
        relay_base_url,
        tunnel_secret: tunnel_secret.clone(),
    };

    cache_tunnel(&state, &tunnel_id, &resp).await;

    Ok(resp)
}

async fn revoke_mobile_access(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Response {
    let token = match extract_bearer(&headers) {
        Some(token) => token,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ApiErrorResp {
                    error: "authorization required".into(),
                }),
            )
                .into_response()
        }
    };

    let user_id = match fetch_user_id(&state, &token).await {
        Ok(id) => id,
        Err((status, msg)) => return (status, Json(ApiErrorResp { error: msg })).into_response(),
    };

    let result = sqlx::query(
        r#"UPDATE mobile_tunnels
           SET disabled_at = CURRENT_TIMESTAMP
           WHERE user_id = ? AND disabled_at IS NULL"#,
    )
    .bind(&user_id)
    .execute(&state.db)
    .await;

    match result {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => {
            warn!("failed to revoke tunnels: {err}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to revoke tunnel".into(),
                }),
            )
                .into_response()
        }
    }
}

async fn load_existing_tunnel(
    state: &AppState,
    user_id: &str,
) -> Result<Option<EnableMobileAccessResp>, (StatusCode, String)> {
    let row = sqlx::query(
        r#"SELECT tunnel_id, relay_base_url, public_base_url
           FROM mobile_tunnels
           WHERE user_id = ? AND disabled_at IS NULL
           ORDER BY created_at DESC
           LIMIT 1"#,
    )
    .bind(user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        warn!("failed to load tunnel: {e}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to load tunnel".to_string(),
        )
    })?;

    let Some(row) = row else {
        return Ok(None);
    };
    let tunnel_id: String = row.try_get("tunnel_id").map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid tunnel record".to_string(),
        )
    })?;
    let relay_base_url: String = row.try_get("relay_base_url").map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid tunnel record".to_string(),
        )
    })?;
    let public_base_url: String = row.try_get("public_base_url").map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid tunnel record".to_string(),
        )
    })?;
    let public_base_url = normalize_public_base_url(&public_base_url, &tunnel_id);
    let tunnel_secret = derive_tunnel_secret(&state.config.master_secret, &tunnel_id)?;
    Ok(Some(EnableMobileAccessResp {
        tunnel_id,
        relay_base_url,
        public_base_url,
        tunnel_secret,
    }))
}

async fn fetch_user_id(state: &AppState, token: &str) -> Result<String, (StatusCode, String)> {
    let url = format!(
        "{}/auth/v1/user",
        state.config.supabase_url.trim_end_matches('/')
    );
    let resp = state
        .client
        .get(url)
        .header("apikey", &state.config.supabase_anon_key)
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .send()
        .await
        .map_err(|_| {
            (
                StatusCode::BAD_GATEWAY,
                "auth service unavailable".to_string(),
            )
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        warn!("auth user failed status={status} body={body}");
        return Err((StatusCode::UNAUTHORIZED, "invalid user session".to_string()));
    }

    let value = resp
        .json::<serde_json::Value>()
        .await
        .map_err(|_| (StatusCode::BAD_GATEWAY, "invalid auth response".to_string()))?;
    let user_id = value
        .get("user")
        .and_then(|user| user.get("id"))
        .or_else(|| value.get("id"))
        .and_then(|id| id.as_str());
    let Some(user_id) = user_id else {
        return Err((StatusCode::UNAUTHORIZED, "invalid user session".to_string()));
    };
    Ok(user_id.to_string())
}

async fn ensure_entitled(state: &AppState, token: &str) -> Result<(), (StatusCode, String)> {
    let url = Url::parse(&state.config.entitlements_url).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid entitlements url".to_string(),
        )
    })?;
    let resp = state
        .client
        .get(url)
        .header("apikey", &state.config.supabase_anon_key)
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .send()
        .await
        .map_err(|_| {
            (
                StatusCode::BAD_GATEWAY,
                "entitlements unavailable".to_string(),
            )
        })?;

    if !resp.status().is_success() {
        return Err((StatusCode::FORBIDDEN, "entitlements required".to_string()));
    }

    let snapshot = resp.json::<EntitlementsSnapshot>().await.map_err(|_| {
        (
            StatusCode::BAD_GATEWAY,
            "invalid entitlements response".to_string(),
        )
    })?;

    let enabled = snapshot
        .features
        .get("remote_mobile_access")
        .and_then(|v| v.as_str())
        .map(|v| v == "enabled")
        .unwrap_or(false);

    if !enabled {
        return Err((
            StatusCode::FORBIDDEN,
            "mobile access not entitled".to_string(),
        ));
    }
    Ok(())
}

fn extract_bearer(headers: &axum::http::HeaderMap) -> Option<String> {
    let value = headers.get(AUTHORIZATION)?.to_str().ok()?;
    value.strip_prefix("Bearer ").map(|v| v.trim().to_string())
}

fn assign_relay(relays: &[String], user_id: &str) -> String {
    let mut hasher = DefaultHasher::new();
    user_id.hash(&mut hasher);
    let idx = (hasher.finish() as usize) % relays.len();
    relays[idx].clone()
}

fn normalize_public_base_url(base: &str, tunnel_id: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    if trimmed.contains("/t/") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/t/{tunnel_id}")
    }
}

fn derive_tunnel_secret(master: &[u8], tunnel_id: &str) -> Result<String, (StatusCode, String)> {
    let mut mac = Hmac::<Sha256>::new_from_slice(master).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid master secret".to_string(),
        )
    })?;
    mac.update(tunnel_id.as_bytes());
    let digest = mac.finalize().into_bytes();
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
    Ok(encoded)
}

async fn cache_tunnel(state: &AppState, tunnel_id: &str, resp: &EnableMobileAccessResp) {
    let Some(mut conn) = state.redis.clone() else {
        return;
    };
    let json = match serde_json::to_string(&TunnelCacheEntry {
        relay_base_url: resp.relay_base_url.clone(),
        public_base_url: resp.public_base_url.clone(),
    }) {
        Ok(json) => json,
        Err(_) => return,
    };
    let key = format!("tunnel:{tunnel_id}");
    let _: redis::RedisResult<()> = redis::AsyncCommands::set_ex(&mut conn, key, json, 300).await;
}

#[derive(Debug, Serialize, Deserialize)]
struct TunnelCacheEntry {
    relay_base_url: String,
    public_base_url: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get;
    use axum::Router;
    use std::net::{Ipv4Addr, SocketAddr};
    use tokio::sync::oneshot;

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
        let migrator = sqlx::migrate::Migrator::new(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations"),
        )
        .await
        .expect("load migrations");
        migrator.run(&db).await.expect("run migrations");

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
            redis: None,
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
}
