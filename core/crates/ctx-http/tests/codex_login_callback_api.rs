use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use ctx_http::api;
use ctx_http::daemon::AppState;
use ctx_provider_accounts::{
    save_codex_registry, CodexAccountEntry, CodexAccountRegistry, CodexEndpointProfile,
    CodexLoginStatus, CODEX_API_SHAPE_OPENAI_RESPONSES, CODEX_CREDENTIAL_KIND_API_KEY,
};
use ctx_providers::adapters::ProviderAdapter;
use ctx_providers::fake::FakeProviderAdapter;
use ctx_store::StoreManager;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
struct CompleteResp {
    accepted: bool,
    status_code: u16,
}

#[derive(Debug, Deserialize)]
struct ErrorResp {
    error: String,
}

async fn start_http_app(
    state: Arc<AppState>,
) -> (String, reqwest::Client, tokio::task::JoinHandle<()>) {
    let app = api::router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), reqwest::Client::new(), handle)
}

async fn start_callback_server() -> (String, tokio::task::JoinHandle<()>) {
    let app = axum::Router::new().route(
        "/auth/callback",
        axum::routing::get(|| async { StatusCode::OK }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://127.0.0.1:{}", addr.port()), handle)
}

async fn start_delayed_callback_server(
    delay: Duration,
) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let route_hits = hits.clone();
    let app = axum::Router::new().route(
        "/auth/callback",
        axum::routing::get(move || {
            let route_hits = route_hits.clone();
            async move {
                route_hits.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(delay).await;
                StatusCode::OK
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://127.0.0.1:{}", addr.port()), hits, handle)
}

async fn start_redirecting_callback_server(
) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let redirected_hits = Arc::new(AtomicUsize::new(0));
    let route_hits = redirected_hits.clone();
    let app = axum::Router::new()
        .route(
            "/auth/callback",
            axum::routing::get(|| async {
                (
                    StatusCode::FOUND,
                    [(axum::http::header::LOCATION, "/redirected")],
                )
            }),
        )
        .route(
            "/redirected",
            axum::routing::get(move || {
                let route_hits = route_hits.clone();
                async move {
                    route_hits.fetch_add(1, Ordering::SeqCst);
                    StatusCode::OK
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (
        format!("http://127.0.0.1:{}", addr.port()),
        redirected_hits,
        handle,
    )
}

async fn app_state(data_root: &std::path::Path) -> Arc<AppState> {
    let stores = StoreManager::open(data_root).await.unwrap();
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));
    Arc::new(AppState::new(
        data_root.to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0".to_string(),
        None,
    ))
}

async fn insert_pending_login(
    state: &Arc<AppState>,
    account_id: &str,
    completion_token: &str,
    expected_callback_url: &str,
) {
    state
        .providers
        .with_codex_login_sessions(|map| {
            map.insert(
                account_id.to_string(),
                CodexLoginStatus {
                    account_id: account_id.to_string(),
                    auth_url: "https://chat.openai.com/oauth/authorize".to_string(),
                    expected_callback_url: Some(expected_callback_url.to_string()),
                    completion_token: Some(completion_token.to_string()),
                    status: "pending".to_string(),
                    error: None,
                },
            );
        })
        .await;
}

#[tokio::test]
async fn complete_login_replays_loopback_callback_and_clears_token() {
    let data_dir = tempfile::tempdir().unwrap();
    let state = app_state(data_dir.path()).await;
    let (callback_base, callback_handle) = start_callback_server().await;
    let expected_callback = format!("{callback_base}/auth/callback");
    let callback_url = format!("{expected_callback}?code=abc&state=xyz");
    let account_id = "acct-success";
    let token = "completion-token";
    insert_pending_login(&state, account_id, token, &expected_callback).await;

    let (base, client, server_handle) = start_http_app(state.clone()).await;
    let resp = client
        .post(format!(
            "{base}/api/providers/codex/accounts/login/{account_id}"
        ))
        .json(&json!({
            "callback_url": callback_url,
            "completion_token": token
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: CompleteResp = resp.json().await.unwrap();
    assert!(body.accepted);
    assert_eq!(body.status_code, 200);

    let status = state
        .providers
        .with_codex_login_sessions(|map| map.get(account_id).cloned())
        .await
        .expect("login status");
    assert!(
        status.completion_token.is_none(),
        "completion token should be single-use"
    );

    callback_handle.abort();
    server_handle.abort();
}

#[tokio::test]
async fn complete_login_replays_localhost_callback_via_ipv4_override() {
    let data_dir = tempfile::tempdir().unwrap();
    let state = app_state(data_dir.path()).await;
    let (callback_base, callback_handle) = start_callback_server().await;
    let callback_port = callback_base
        .rsplit_once(':')
        .expect("callback base port")
        .1
        .parse::<u16>()
        .expect("callback port");
    let expected_callback = format!("http://localhost:{callback_port}/auth/callback");
    let callback_url = format!("{expected_callback}?code=abc&state=xyz");
    let account_id = "acct-localhost";
    let token = "completion-token";
    insert_pending_login(&state, account_id, token, &expected_callback).await;

    let (base, client, server_handle) = start_http_app(state.clone()).await;
    let resp = client
        .post(format!(
            "{base}/api/providers/codex/accounts/login/{account_id}"
        ))
        .json(&json!({
            "callback_url": callback_url,
            "completion_token": token
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: CompleteResp = resp.json().await.unwrap();
    assert!(body.accepted);
    assert_eq!(body.status_code, 200);

    callback_handle.abort();
    server_handle.abort();
}

#[tokio::test]
async fn complete_login_rejects_invalid_completion_token() {
    let data_dir = tempfile::tempdir().unwrap();
    let state = app_state(data_dir.path()).await;
    let expected_callback = "http://localhost:43210/auth/callback";
    insert_pending_login(&state, "acct-token", "expected-token", expected_callback).await;
    let (base, client, server_handle) = start_http_app(state).await;

    let resp = client
        .post(format!(
            "{base}/api/providers/codex/accounts/login/acct-token"
        ))
        .json(&json!({
            "callback_url": "http://localhost:43210/auth/callback?code=abc",
            "completion_token": "wrong-token"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let body: ErrorResp = resp.json().await.unwrap();
    assert!(body.error.contains("invalid completion token"));

    server_handle.abort();
}

#[tokio::test]
async fn complete_login_rejects_missing_expected_callback_metadata() {
    let data_dir = tempfile::tempdir().unwrap();
    let state = app_state(data_dir.path()).await;
    let (callback_base, callback_hits, callback_handle) =
        start_delayed_callback_server(Duration::from_millis(0)).await;
    let callback_url = format!("{callback_base}/auth/callback?code=abc");
    let account_id = "acct-missing-expected-callback";
    let token = "token-missing-expected-callback";
    state
        .providers
        .with_codex_login_sessions(|map| {
            map.insert(
                account_id.to_string(),
                CodexLoginStatus {
                    account_id: account_id.to_string(),
                    auth_url: "https://chat.openai.com/oauth/authorize".to_string(),
                    expected_callback_url: None,
                    completion_token: Some(token.to_string()),
                    status: "pending".to_string(),
                    error: None,
                },
            );
        })
        .await;
    let (base, client, server_handle) = start_http_app(state.clone()).await;

    let resp = client
        .post(format!(
            "{base}/api/providers/codex/accounts/login/{account_id}"
        ))
        .json(&json!({
            "callback_url": callback_url,
            "completion_token": token
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let body: ErrorResp = resp.json().await.unwrap();
    assert!(body.error.contains("expected callback"));

    let status = state
        .providers
        .with_codex_login_sessions(|map| map.get(account_id).cloned())
        .await
        .expect("login status");
    assert_eq!(status.completion_token.as_deref(), Some(token));
    assert_eq!(callback_hits.load(Ordering::SeqCst), 0);

    callback_handle.abort();
    server_handle.abort();
}

#[tokio::test]
async fn complete_login_rejects_non_loopback_host() {
    let data_dir = tempfile::tempdir().unwrap();
    let state = app_state(data_dir.path()).await;
    insert_pending_login(
        &state,
        "acct-host",
        "token-host",
        "http://localhost:12345/auth/callback",
    )
    .await;
    let (base, client, server_handle) = start_http_app(state).await;

    let resp = client
        .post(format!(
            "{base}/api/providers/codex/accounts/login/acct-host"
        ))
        .json(&json!({
            "callback_url": "http://example.com:12345/auth/callback?code=abc",
            "completion_token": "token-host"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body: ErrorResp = resp.json().await.unwrap();
    assert!(body.error.contains("loopback"));

    server_handle.abort();
}

#[tokio::test]
async fn complete_login_rejects_expected_path_mismatch() {
    let data_dir = tempfile::tempdir().unwrap();
    let state = app_state(data_dir.path()).await;
    insert_pending_login(
        &state,
        "acct-path",
        "token-path",
        "http://localhost:24567/auth/callback",
    )
    .await;
    let (base, client, server_handle) = start_http_app(state).await;

    let resp = client
        .post(format!(
            "{base}/api/providers/codex/accounts/login/acct-path"
        ))
        .json(&json!({
            "callback_url": "http://localhost:24567/auth/other?code=abc",
            "completion_token": "token-path"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body: ErrorResp = resp.json().await.unwrap();
    assert!(body.error.contains("path"));

    server_handle.abort();
}

#[tokio::test]
async fn complete_login_accepts_loopback_alias_for_expected_host() {
    let data_dir = tempfile::tempdir().unwrap();
    let state = app_state(data_dir.path()).await;
    let (callback_base, callback_handle) = start_callback_server().await;
    let expected_callback = format!("{callback_base}/auth/callback");
    let parsed = reqwest::Url::parse(&expected_callback).unwrap();
    let port = parsed.port().unwrap();
    insert_pending_login(
        &state,
        "acct-host-mismatch",
        "token-host-mismatch",
        &expected_callback,
    )
    .await;
    let (base, client, server_handle) = start_http_app(state).await;

    let resp = client
        .post(format!(
            "{base}/api/providers/codex/accounts/login/acct-host-mismatch"
        ))
        .json(&json!({
            "callback_url": format!("http://localhost:{port}/auth/callback?code=abc"),
            "completion_token": "token-host-mismatch"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    callback_handle.abort();
    server_handle.abort();
}

#[tokio::test]
async fn complete_login_token_is_single_use() {
    let data_dir = tempfile::tempdir().unwrap();
    let state = app_state(data_dir.path()).await;
    let (callback_base, callback_handle) = start_callback_server().await;
    let expected_callback = format!("{callback_base}/auth/callback");
    let callback_url = format!("{expected_callback}?code=one-time");
    insert_pending_login(&state, "acct-replay", "token-replay", &expected_callback).await;

    let (base, client, server_handle) = start_http_app(state).await;
    let first = client
        .post(format!(
            "{base}/api/providers/codex/accounts/login/acct-replay"
        ))
        .json(&json!({
            "callback_url": callback_url,
            "completion_token": "token-replay"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);

    let second = client
        .post(format!(
            "{base}/api/providers/codex/accounts/login/acct-replay"
        ))
        .json(&json!({
            "callback_url": "http://localhost:1/auth/callback?code=replay",
            "completion_token": "token-replay"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::UNAUTHORIZED);
    let body: ErrorResp = second.json().await.unwrap();
    assert!(body.error.contains("invalid completion token"));

    callback_handle.abort();
    server_handle.abort();
}

#[tokio::test]
async fn complete_login_allows_only_one_concurrent_callback_replay() {
    let data_dir = tempfile::tempdir().unwrap();
    let state = app_state(data_dir.path()).await;
    let (callback_base, callback_hits, callback_handle) =
        start_delayed_callback_server(Duration::from_millis(250)).await;
    let expected_callback = format!("{callback_base}/auth/callback");
    let callback_url = format!("{expected_callback}?code=race");
    insert_pending_login(&state, "acct-race", "token-race", &expected_callback).await;

    let (base, client, server_handle) = start_http_app(state).await;
    let request_url = format!("{base}/api/providers/codex/accounts/login/acct-race");
    let payload = json!({
        "callback_url": callback_url,
        "completion_token": "token-race"
    });

    let first = client.post(request_url.clone()).json(&payload).send();
    let second = client.post(request_url).json(&payload).send();
    let (first, second) = tokio::join!(first, second);
    let statuses = [first.unwrap().status(), second.unwrap().status()];

    assert!(statuses.contains(&StatusCode::OK));
    assert!(statuses.contains(&StatusCode::UNAUTHORIZED));
    assert_eq!(callback_hits.load(Ordering::SeqCst), 1);

    callback_handle.abort();
    server_handle.abort();
}

#[tokio::test]
async fn complete_login_rejects_redirecting_callback_replay() {
    let data_dir = tempfile::tempdir().unwrap();
    let state = app_state(data_dir.path()).await;
    let (callback_base, redirected_hits, callback_handle) =
        start_redirecting_callback_server().await;
    let expected_callback = format!("{callback_base}/auth/callback");
    let callback_url = format!("{expected_callback}?code=redirect");
    insert_pending_login(
        &state,
        "acct-redirect",
        "token-redirect",
        &expected_callback,
    )
    .await;

    let (base, client, server_handle) = start_http_app(state).await;
    let resp = client
        .post(format!(
            "{base}/api/providers/codex/accounts/login/acct-redirect"
        ))
        .json(&json!({
            "callback_url": callback_url,
            "completion_token": "token-redirect"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    let body: ErrorResp = resp.json().await.unwrap();
    assert!(body.error.contains("302"));
    assert_eq!(redirected_hits.load(Ordering::SeqCst), 0);

    callback_handle.abort();
    server_handle.abort();
}

#[tokio::test]
async fn set_active_account_rejects_incompatible_endpoint_profile() {
    let data_dir = tempfile::tempdir().unwrap();
    let state = app_state(data_dir.path()).await;
    let registry = CodexAccountRegistry {
        active_account_id: None,
        accounts: vec![CodexAccountEntry {
            id: "acct-incompatible".to_string(),
            label: "Incompatible".to_string(),
            kind: CODEX_CREDENTIAL_KIND_API_KEY.to_string(),
            email: None,
            provider_account_id: None,
            plan_type: None,
            created_at: chrono::Utc::now(),
            last_used_at: None,
            secret_ref: None,
            endpoint_profile: CodexEndpointProfile {
                api_shape: CODEX_API_SHAPE_OPENAI_RESPONSES.to_string(),
                auth_type: "basic".to_string(),
                base_url: Some("https://example.com/v1".to_string()),
            },
        }],
    };
    save_codex_registry(data_dir.path(), &registry)
        .await
        .unwrap();

    let (base, client, server_handle) = start_http_app(state).await;
    let resp = client
        .put(format!("{base}/api/providers/codex/active-account"))
        .json(&json!({ "account_id": "acct-incompatible" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body: ErrorResp = resp.json().await.unwrap();
    assert!(body.error.contains("auth_type=bearer"));

    server_handle.abort();
}
