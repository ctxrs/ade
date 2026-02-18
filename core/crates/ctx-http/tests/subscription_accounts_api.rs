mod common;

use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
struct SubscriptionAccountEntry {
    id: String,
}

#[derive(Debug, Deserialize)]
struct SubscriptionAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<SubscriptionAccountEntry>,
}

#[derive(Debug, Deserialize)]
struct ErrorResp {
    error: String,
}

async fn assert_managed_subscription_crud(provider_id: &str, upsert_body: serde_json::Value) {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let server = common::spawn_http_server(common::router(state)).await;

    let accounts_url = format!("{}/api/providers/{provider_id}/accounts", server.base_url);
    let active_url = format!(
        "{}/api/providers/{provider_id}/active-account",
        server.base_url
    );

    let created = server
        .client
        .post(&accounts_url)
        .json(&upsert_body)
        .send()
        .await
        .expect("create account request");
    assert_eq!(created.status(), StatusCode::OK);
    let created_body: SubscriptionAccountsResponse =
        created.json().await.expect("created response json");
    assert_eq!(created_body.accounts.len(), 1);
    let account_id = created_body.accounts[0].id.clone();
    assert_eq!(
        created_body.active_account_id.as_deref(),
        Some(account_id.as_str())
    );

    let listed = server
        .client
        .get(&accounts_url)
        .send()
        .await
        .expect("list accounts request");
    assert_eq!(listed.status(), StatusCode::OK);
    let listed_body: SubscriptionAccountsResponse =
        listed.json().await.expect("list response json");
    assert_eq!(listed_body.accounts.len(), 1);
    assert_eq!(listed_body.accounts[0].id, account_id);

    let missing = server
        .client
        .put(&active_url)
        .json(&json!({ "account_id": "missing-account" }))
        .send()
        .await
        .expect("set active missing request");
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    let activated = server
        .client
        .put(&active_url)
        .json(&json!({ "account_id": account_id }))
        .send()
        .await
        .expect("set active request");
    assert_eq!(activated.status(), StatusCode::OK);
    let activated_body: SubscriptionAccountsResponse =
        activated.json().await.expect("set active response json");
    assert_eq!(
        activated_body.active_account_id.as_deref(),
        Some(activated_body.accounts[0].id.as_str())
    );

    let deleted = server
        .client
        .delete(format!("{accounts_url}/{}", activated_body.accounts[0].id))
        .send()
        .await
        .expect("delete account request");
    assert_eq!(deleted.status(), StatusCode::OK);
    let deleted_body: SubscriptionAccountsResponse =
        deleted.json().await.expect("delete response json");
    assert!(deleted_body.accounts.is_empty());
    assert!(deleted_body.active_account_id.is_none());
}

#[tokio::test]
async fn claude_subscription_accounts_crud_round_trip() {
    assert_managed_subscription_crud(
        "claude-crp",
        json!({
            "label": "Claude Team",
            "auth_token": "token-abc"
        }),
    )
    .await;
}

#[tokio::test]
async fn gemini_subscription_accounts_crud_round_trip() {
    assert_managed_subscription_crud(
        "gemini",
        json!({
            "label": "Gemini Team",
            "oauth_creds_json": "{\"access_token\":\"a\",\"refresh_token\":\"b\"}",
            "google_accounts_json": "[{\"email\":\"dev@example.com\"}]",
            "email": "dev@example.com"
        }),
    )
    .await;
}

#[tokio::test]
async fn kimi_subscription_accounts_crud_round_trip() {
    assert_managed_subscription_crud(
        "kimi",
        json!({
            "label": "Kimi Team",
            "provider": "moonshot",
            "credentials_json": "{\"access_token\":\"a\",\"refresh_token\":\"b\"}",
            "config_toml": "current_provider = \"moonshot\"",
            "email": "dev@example.com"
        }),
    )
    .await;
}

#[tokio::test]
async fn copilot_subscription_accounts_crud_round_trip() {
    assert_managed_subscription_crud(
        "copilot",
        json!({
            "label": "Copilot Team",
            "token": "ghp_abc",
            "email": "dev@example.com"
        }),
    )
    .await;
}

#[tokio::test]
async fn kiro_subscription_accounts_crud_round_trip() {
    assert_managed_subscription_crud(
        "kiro",
        json!({
            "label": "Kiro Team",
            "auth_token_json": "{\"accessToken\":\"a\",\"expiresAt\":\"2099-01-01T00:00:00Z\"}",
            "email": "dev@example.com"
        }),
    )
    .await;
}

#[tokio::test]
async fn cursor_subscription_accounts_crud_round_trip() {
    assert_managed_subscription_crud(
        "cursor",
        json!({
            "label": "Cursor Team",
            "token": "cursor-key",
            "email": "dev@example.com"
        }),
    )
    .await;
}

#[tokio::test]
async fn gemini_upsert_rejects_invalid_oauth_json() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let server = common::spawn_http_server(common::router(state)).await;
    let accounts_url = format!("{}/api/providers/gemini/accounts", server.base_url);

    let resp = server
        .client
        .post(accounts_url)
        .json(&json!({
            "label": "Gemini Bad",
            "oauth_creds_json": "not-json"
        }))
        .send()
        .await
        .expect("invalid upsert request");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body: ErrorResp = resp.json().await.expect("error response json");
    assert!(body.error.contains("valid JSON"));
}

#[tokio::test]
async fn kimi_upsert_rejects_invalid_credentials_json() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let server = common::spawn_http_server(common::router(state)).await;
    let accounts_url = format!("{}/api/providers/kimi/accounts", server.base_url);

    let resp = server
        .client
        .post(accounts_url)
        .json(&json!({
            "label": "Kimi Bad",
            "credentials_json": "not-json"
        }))
        .send()
        .await
        .expect("invalid upsert request");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body: ErrorResp = resp.json().await.expect("error response json");
    assert!(body.error.contains("valid JSON"));
}

#[tokio::test]
async fn kiro_upsert_rejects_invalid_auth_token_json() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let server = common::spawn_http_server(common::router(state)).await;
    let accounts_url = format!("{}/api/providers/kiro/accounts", server.base_url);

    let resp = server
        .client
        .post(accounts_url)
        .json(&json!({
            "label": "Kiro Bad",
            "auth_token_json": "not-json"
        }))
        .send()
        .await
        .expect("invalid upsert request");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body: ErrorResp = resp.json().await.expect("error response json");
    assert!(body.error.contains("valid JSON"));
}
