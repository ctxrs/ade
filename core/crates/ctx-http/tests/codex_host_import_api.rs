use std::collections::HashMap;
use std::sync::Arc;

use axum::http::StatusCode;
use ctx_provider_accounts::{codex_env_for_active_account, ensure_codex_auth_ready};
use ctx_http::api;
use ctx_http::daemon::AppState;
use ctx_providers::adapters::ProviderAdapter;
use ctx_providers::fake::FakeProviderAdapter;
use ctx_store::StoreManager;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
struct CodexEndpointProfile {
    api_shape: String,
    auth_type: String,
}

#[derive(Debug, Deserialize)]
struct CodexAccountEntry {
    id: String,
    label: String,
    kind: String,
    endpoint_profile: CodexEndpointProfile,
}

#[derive(Debug, Deserialize)]
struct CodexAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<CodexAccountEntry>,
}

#[derive(Debug, Deserialize)]
struct CodexHostImportProbe {
    available: bool,
    auth_kind: Option<String>,
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

#[tokio::test]
async fn host_import_probe_and_import_projects_runtime_auth() {
    let data_dir = tempfile::tempdir().unwrap();
    let host_dir = tempfile::tempdir().unwrap();
    let host_auth_path = host_dir.path().join("auth.json");
    tokio::fs::write(&host_auth_path, br#"{"OPENAI_API_KEY":"test-key"}"#)
        .await
        .unwrap();

    let prev = std::env::var("CTX_CODEX_HOST_AUTH_PATH").ok();
    std::env::set_var(
        "CTX_CODEX_HOST_AUTH_PATH",
        host_auth_path.to_string_lossy().as_ref(),
    );

    let state = app_state(data_dir.path()).await;
    let (base, client, server_handle) = start_http_app(state).await;

    let probe = client
        .get(format!("{base}/api/providers/codex/import/host"))
        .send()
        .await
        .unwrap();
    assert_eq!(probe.status(), StatusCode::OK);
    let probe_body: CodexHostImportProbe = probe.json().await.unwrap();
    assert!(probe_body.available);
    assert_eq!(probe_body.auth_kind.as_deref(), Some("api_key"));

    let imported = client
        .post(format!("{base}/api/providers/codex/import/host"))
        .json(&json!({ "label": "Imported Host Auth" }))
        .send()
        .await
        .unwrap();
    assert_eq!(imported.status(), StatusCode::OK);
    let imported_body: CodexAccountsResponse = imported.json().await.unwrap();
    assert_eq!(imported_body.accounts.len(), 1);
    let active_id = imported_body.active_account_id.expect("active account");
    let account = imported_body
        .accounts
        .iter()
        .find(|entry| entry.id == active_id)
        .expect("active account entry");
    assert_eq!(account.label, "Imported Host Auth");
    assert_eq!(account.kind, "api_key");
    assert_eq!(account.endpoint_profile.api_shape, "openai_responses");
    assert_eq!(account.endpoint_profile.auth_type, "bearer");

    let env = codex_env_for_active_account(data_dir.path()).await.unwrap();
    let home = env.get("CODEX_HOME").expect("CODEX_HOME");
    ensure_codex_auth_ready(std::path::Path::new(home))
        .await
        .unwrap();

    server_handle.abort();
    if let Some(value) = prev {
        std::env::set_var("CTX_CODEX_HOST_AUTH_PATH", value);
    } else {
        std::env::remove_var("CTX_CODEX_HOST_AUTH_PATH");
    }
}
