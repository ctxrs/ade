mod common;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use ctx_http::installer::{save_agent_server_config, AgentServerCommand, AgentServerConfigFile};

#[derive(Debug, Deserialize)]
struct SubscriptionAccountEntry {
    id: String,
    label: Option<String>,
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

#[derive(Debug, Deserialize)]
struct ClaudeLoginStartResponse {
    login_id: String,
    auth_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClaudeLoginStatusResponse {
    status: String,
    account_id: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClaudeLoginCompleteResponse {
    accepted: bool,
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

async fn write_mock_claude_runtime(
    data_root: &std::path::Path,
    script_contents: &str,
) -> std::path::PathBuf {
    let script_path = data_root.join("mock-claude");
    tokio::fs::write(&script_path, script_contents)
        .await
        .expect("write mock claude script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&script_path)
            .expect("mock claude metadata")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script_path, perms).expect("set mock claude permissions");
    }
    script_path
}

async fn poll_claude_login_status(
    server: &common::TestServer,
    login_id: &str,
    timeout: Duration,
) -> ClaudeLoginStatusResponse {
    let status_url = format!(
        "{}/api/providers/claude-crp/accounts/login/{}",
        server.base_url, login_id
    );
    let deadline = Instant::now() + timeout;
    loop {
        let resp = server
            .client
            .get(&status_url)
            .send()
            .await
            .expect("claude login status request");
        assert_eq!(resp.status(), StatusCode::OK);
        let body: ClaudeLoginStatusResponse = resp.json().await.expect("claude login status body");
        if body.status != "pending" {
            return body;
        }
        if Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("claude login did not reach terminal status in time");
}

#[tokio::test]
async fn claude_subscription_accounts_crud_round_trip() {
    assert_managed_subscription_crud(
        "claude-crp",
        json!({
            "label": "Claude Team",
            "setup_token": "sk-ant-oat01-abcDEF1234567890_abcdefghijklmnopqrstuvwxyz_0123456789"
        }),
    )
    .await;
}

#[tokio::test]
async fn claude_login_start_and_status_success_persists_account() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let server = common::spawn_http_server(common::router(state)).await;

    let script_path = write_mock_claude_runtime(
        data_dir.path(),
        r#"#!/usr/bin/env bash
set -euo pipefail
if [[ "${1-}" != "--shim" || "${2-}" != "setup-token" ]]; then
  echo "unexpected args: $*" >&2
  exit 2
fi
echo "Claude setup-token URL: https://claude.ai/oauth/authorize?code=test"
echo "Long-lived authentication token created successfully!"
echo ""
echo "Your OAuth token (valid for 1 year):"
echo ""
echo "sk-ant-oat01-abcDEF1234567890_"
echo "ZXY987654321"
"#,
    )
    .await;
    let mut cfg = AgentServerConfigFile {
        providers: HashMap::new(),
        managed_installs: HashMap::new(),
    };
    cfg.providers.insert(
        "claude-cli".to_string(),
        AgentServerCommand {
            command: script_path.to_string_lossy().to_string(),
            args: vec!["--shim".to_string()],
            dependencies: vec![],
            managed: None,
        },
    );
    save_agent_server_config(data_dir.path(), &cfg)
        .await
        .expect("save agent config");

    let start_url = format!(
        "{}/api/providers/claude-crp/accounts/login/start",
        server.base_url
    );
    let start_resp = server
        .client
        .post(start_url)
        .json(&json!({ "label": "Claude OAuth" }))
        .send()
        .await
        .expect("start claude login request");
    assert_eq!(start_resp.status(), StatusCode::OK);
    let start_body: ClaudeLoginStartResponse = start_resp.json().await.expect("start body");
    assert!(!start_body.login_id.is_empty());
    assert!(start_body.auth_url.as_deref().is_some());

    let status =
        poll_claude_login_status(&server, &start_body.login_id, Duration::from_secs(8)).await;
    assert_eq!(status.status, "success");
    assert!(status.account_id.is_some());
    assert!(status.error.is_none());

    let accounts_url = format!("{}/api/providers/claude-crp/accounts", server.base_url);
    let accounts_resp = server
        .client
        .get(accounts_url)
        .send()
        .await
        .expect("claude accounts request");
    assert_eq!(accounts_resp.status(), StatusCode::OK);
    let accounts: SubscriptionAccountsResponse = accounts_resp.json().await.expect("accounts body");
    assert_eq!(accounts.accounts.len(), 1);
    assert_eq!(accounts.active_account_id, status.account_id);
}

#[tokio::test]
async fn claude_login_start_and_status_failure_reports_error() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let server = common::spawn_http_server(common::router(state)).await;

    let script_path = write_mock_claude_runtime(
        data_dir.path(),
        r#"#!/usr/bin/env bash
set -euo pipefail
echo "Claude setup-token URL: https://claude.ai/oauth/authorize?code=test"
echo "failed" >&2
exit 7
"#,
    )
    .await;
    let mut cfg = AgentServerConfigFile {
        providers: HashMap::new(),
        managed_installs: HashMap::new(),
    };
    cfg.providers.insert(
        "claude-cli".to_string(),
        AgentServerCommand {
            command: script_path.to_string_lossy().to_string(),
            args: vec![],
            dependencies: vec![],
            managed: None,
        },
    );
    save_agent_server_config(data_dir.path(), &cfg)
        .await
        .expect("save agent config");

    let start_url = format!(
        "{}/api/providers/claude-crp/accounts/login/start",
        server.base_url
    );
    let start_resp = server
        .client
        .post(start_url)
        .json(&json!({}))
        .send()
        .await
        .expect("start claude login request");
    assert_eq!(start_resp.status(), StatusCode::OK);
    let start_body: ClaudeLoginStartResponse = start_resp.json().await.expect("start body");

    let status =
        poll_claude_login_status(&server, &start_body.login_id, Duration::from_secs(8)).await;
    assert_eq!(status.status, "failed");
    assert!(status.account_id.is_none());
    assert!(status.error.unwrap_or_default().contains("exited"));
}

#[tokio::test]
async fn claude_login_start_reconstructs_wrapped_auth_url() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let server = common::spawn_http_server(common::router(state)).await;

    let script_path = write_mock_claude_runtime(
        data_dir.path(),
        r#"#!/usr/bin/env bash
set -euo pipefail
printf "Claude setup-token URL: https://claude.ai/oauth/authorize?redirect_uri=http%%3A%%2F%%2Flocalhost%%3A\n"
printf "64111%%2Fauth%%2Fcallback&state=test\n"
echo "forced failure after auth url"
exit 5
"#,
    )
    .await;
    let mut cfg = AgentServerConfigFile {
        providers: HashMap::new(),
        managed_installs: HashMap::new(),
    };
    cfg.providers.insert(
        "claude-cli".to_string(),
        AgentServerCommand {
            command: script_path.to_string_lossy().to_string(),
            args: vec![],
            dependencies: vec![],
            managed: None,
        },
    );
    save_agent_server_config(data_dir.path(), &cfg)
        .await
        .expect("save agent config");

    let start_url = format!(
        "{}/api/providers/claude-crp/accounts/login/start",
        server.base_url
    );
    let start_resp = server
        .client
        .post(start_url)
        .json(&json!({}))
        .send()
        .await
        .expect("start claude login request");
    assert_eq!(start_resp.status(), StatusCode::OK);
    let start_body: ClaudeLoginStartResponse = start_resp.json().await.expect("start body");
    assert_eq!(
        start_body.auth_url.as_deref(),
        Some("https://claude.ai/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A64111%2Fauth%2Fcallback&state=test")
    );
}

#[tokio::test]
async fn claude_login_callback_code_completion_path_succeeds() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let server = common::spawn_http_server(common::router(state)).await;

    let script_path = write_mock_claude_runtime(
        data_dir.path(),
        r#"#!/usr/bin/env bash
set -euo pipefail
echo "Claude setup-token URL: https://claude.ai/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A64111%2Fauth%2Fcallback&state=test"
read -r callback_code
if [[ "$callback_code" != *\#* ]]; then
  echo "missing callback code fragment" >&2
  exit 9
fi
echo "Long-lived authentication token created successfully!"
echo ""
echo "Your OAuth token (valid for 1 year):"
echo ""
echo "sk-ant-oat01-abcDEF1234567890_"
echo "ZXY987654321"
"#,
    )
    .await;
    let mut cfg = AgentServerConfigFile {
        providers: HashMap::new(),
        managed_installs: HashMap::new(),
    };
    cfg.providers.insert(
        "claude-cli".to_string(),
        AgentServerCommand {
            command: script_path.to_string_lossy().to_string(),
            args: vec![],
            dependencies: vec![],
            managed: None,
        },
    );
    save_agent_server_config(data_dir.path(), &cfg)
        .await
        .expect("save agent config");

    let start_url = format!(
        "{}/api/providers/claude-crp/accounts/login/start",
        server.base_url
    );
    let start_resp = server
        .client
        .post(&start_url)
        .json(&json!({}))
        .send()
        .await
        .expect("start claude login request");
    assert_eq!(start_resp.status(), StatusCode::OK);
    let start_body: ClaudeLoginStartResponse = start_resp.json().await.expect("start body");
    assert_eq!(
        start_body.auth_url.as_deref(),
        Some("https://claude.ai/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A64111%2Fauth%2Fcallback&state=test")
    );

    let complete_url = format!(
        "{}/api/providers/claude-crp/accounts/login/{}",
        server.base_url, start_body.login_id
    );
    let complete_resp = server
        .client
        .post(&complete_url)
        .json(&json!({ "callback_code": "ePBMdWetJlSbZ0aR#state" }))
        .send()
        .await
        .expect("complete claude login request");
    assert_eq!(complete_resp.status(), StatusCode::OK);
    let complete_body: ClaudeLoginCompleteResponse =
        complete_resp.json().await.expect("complete body");
    assert!(complete_body.accepted);

    let status =
        poll_claude_login_status(&server, &start_body.login_id, Duration::from_secs(8)).await;
    assert_eq!(status.status, "success");
    assert!(status.account_id.is_some());
    assert!(status.error.is_none());
}

#[tokio::test]
async fn claude_login_success_without_token_reports_actionable_error() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let server = common::spawn_http_server(common::router(state)).await;

    let script_path = write_mock_claude_runtime(
        data_dir.path(),
        r#"#!/usr/bin/env bash
set -euo pipefail
echo "Claude setup-token URL: https://claude.ai/oauth/authorize?code=test"
echo "Long-lived authentication token created successfully!"
echo "Token omitted intentionally for test."
"#,
    )
    .await;
    let mut cfg = AgentServerConfigFile {
        providers: HashMap::new(),
        managed_installs: HashMap::new(),
    };
    cfg.providers.insert(
        "claude-cli".to_string(),
        AgentServerCommand {
            command: script_path.to_string_lossy().to_string(),
            args: vec![],
            dependencies: vec![],
            managed: None,
        },
    );
    save_agent_server_config(data_dir.path(), &cfg)
        .await
        .expect("save agent config");

    let start_url = format!(
        "{}/api/providers/claude-crp/accounts/login/start",
        server.base_url
    );
    let start_resp = server
        .client
        .post(start_url)
        .json(&json!({}))
        .send()
        .await
        .expect("start claude login request");
    assert_eq!(start_resp.status(), StatusCode::OK);
    let start_body: ClaudeLoginStartResponse = start_resp.json().await.expect("start body");

    let status =
        poll_claude_login_status(&server, &start_body.login_id, Duration::from_secs(8)).await;
    assert_eq!(status.status, "failed");
    assert!(status.account_id.is_none());
    assert!(status
        .error
        .unwrap_or_default()
        .contains("no setup token was detected"));
}

#[tokio::test]
async fn claude_login_hang_without_auth_url_times_out_and_fails() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let server = common::spawn_http_server(common::router(state)).await;

    let script_path = write_mock_claude_runtime(
        data_dir.path(),
        r#"#!/usr/bin/env bash
set -euo pipefail
sleep 30
"#,
    )
    .await;
    let mut cfg = AgentServerConfigFile {
        providers: HashMap::new(),
        managed_installs: HashMap::new(),
    };
    cfg.providers.insert(
        "claude-cli".to_string(),
        AgentServerCommand {
            command: script_path.to_string_lossy().to_string(),
            args: vec![],
            dependencies: vec![],
            managed: None,
        },
    );
    save_agent_server_config(data_dir.path(), &cfg)
        .await
        .expect("save agent config");

    let start_url = format!(
        "{}/api/providers/claude-crp/accounts/login/start",
        server.base_url
    );
    let start_resp = server
        .client
        .post(start_url)
        .json(&json!({}))
        .send()
        .await
        .expect("start claude login request");
    assert_eq!(start_resp.status(), StatusCode::OK);
    let start_body: ClaudeLoginStartResponse = start_resp.json().await.expect("start body");
    assert!(start_body.auth_url.is_none());

    let status =
        poll_claude_login_status(&server, &start_body.login_id, Duration::from_secs(20)).await;
    assert_eq!(status.status, "failed");
    assert!(status.account_id.is_none());
    assert!(status
        .error
        .unwrap_or_default()
        .contains("did not emit an authentication URL"));
}

#[tokio::test]
async fn claude_login_without_label_preserves_existing_account_label() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let server = common::spawn_http_server(common::router(state)).await;

    let shared_token = "sk-ant-oat01-abcDEF1234567890_abcdefghijklmnopqrstuvwxyz_0123456789";
    let accounts_url = format!("{}/api/providers/claude-crp/accounts", server.base_url);
    let existing_resp = server
        .client
        .post(&accounts_url)
        .json(&json!({
            "label": "Claude Existing Label",
            "setup_token": shared_token
        }))
        .send()
        .await
        .expect("create existing claude account request");
    assert_eq!(existing_resp.status(), StatusCode::OK);
    let existing_body: SubscriptionAccountsResponse = existing_resp
        .json()
        .await
        .expect("existing claude account body");
    let existing_account = existing_body
        .accounts
        .first()
        .expect("existing account should be present");
    let existing_id = existing_account.id.clone();
    assert_eq!(
        existing_account.label.as_deref(),
        Some("Claude Existing Label")
    );

    let script_with_token = format!(
        r#"#!/usr/bin/env bash
set -euo pipefail
echo "Claude setup-token URL: https://claude.ai/oauth/authorize?code=test"
echo "Long-lived authentication token created successfully!"
echo ""
echo "Your OAuth token (valid for 1 year):"
echo ""
echo "{}"
"#,
        shared_token
    );
    let script_path = write_mock_claude_runtime(data_dir.path(), &script_with_token).await;
    let mut cfg = AgentServerConfigFile {
        providers: HashMap::new(),
        managed_installs: HashMap::new(),
    };
    cfg.providers.insert(
        "claude-cli".to_string(),
        AgentServerCommand {
            command: script_path.to_string_lossy().to_string(),
            args: vec![],
            dependencies: vec![],
            managed: None,
        },
    );
    save_agent_server_config(data_dir.path(), &cfg)
        .await
        .expect("save agent config");

    let start_url = format!(
        "{}/api/providers/claude-crp/accounts/login/start",
        server.base_url
    );
    let start_resp = server
        .client
        .post(start_url)
        .json(&json!({}))
        .send()
        .await
        .expect("start claude login request");
    assert_eq!(start_resp.status(), StatusCode::OK);
    let start_body: ClaudeLoginStartResponse = start_resp.json().await.expect("start body");

    let status =
        poll_claude_login_status(&server, &start_body.login_id, Duration::from_secs(8)).await;
    assert_eq!(status.status, "success");
    assert_eq!(status.account_id.as_deref(), Some(existing_id.as_str()));

    let listed_resp = server
        .client
        .get(&accounts_url)
        .send()
        .await
        .expect("list claude accounts request");
    assert_eq!(listed_resp.status(), StatusCode::OK);
    let listed_body: SubscriptionAccountsResponse =
        listed_resp.json().await.expect("list claude accounts body");
    assert_eq!(listed_body.accounts.len(), 1);
    assert_eq!(listed_body.accounts[0].id, existing_id);
    assert_eq!(
        listed_body.accounts[0].label.as_deref(),
        Some("Claude Existing Label")
    );
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
async fn gemini_login_start_and_status_success_persists_account() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let providers = providers_with_gemini_adapter(Arc::new(GeminiLoginTestAdapter::success(
        r#"{"access_token":"access","refresh_token":"refresh"}"#,
        Some(r#"[{"email":"gemini-dev@example.com"}]"#.to_string()),
        Some("https://accounts.google.com/o/oauth2/auth?code=test".to_string()),
    )));
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0",
    );
    let server = common::spawn_http_server(common::router(state)).await;

    let start_url = format!(
        "{}/api/providers/gemini/accounts/login/start",
        server.base_url
    );
    let start_resp = server
        .client
        .post(start_url)
        .json(&json!({ "label": "Gemini OAuth" }))
        .send()
        .await
        .expect("start gemini login request");
    assert_eq!(start_resp.status(), StatusCode::OK);
    let start_body: GeminiLoginStartResponse = start_resp.json().await.expect("start body");
    assert!(!start_body.login_id.is_empty());
    assert!(start_body.auth_url.is_none());

    let status = poll_gemini_login_status(&server, &start_body.login_id).await;
    assert_eq!(status.status, "success");
    assert!(status.account_id.is_some());
    assert!(status.error.is_none());
    assert!(status.auth_url.as_deref().is_some());

    let accounts_url = format!("{}/api/providers/gemini/accounts", server.base_url);
    let accounts_resp = server
        .client
        .get(accounts_url)
        .send()
        .await
        .expect("gemini accounts request");
    assert_eq!(accounts_resp.status(), StatusCode::OK);
    let accounts: SubscriptionAccountsResponse = accounts_resp.json().await.expect("accounts body");
    assert_eq!(accounts.accounts.len(), 1);
    assert_eq!(accounts.active_account_id, status.account_id);
    assert_eq!(accounts.accounts[0].label.as_deref(), Some("Gemini OAuth"));
}

#[tokio::test]
async fn gemini_login_start_and_status_failure_reports_error() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let providers = providers_with_gemini_adapter(Arc::new(GeminiLoginTestAdapter::failure(
        "gemini auth failed",
    )));
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0",
    );
    let server = common::spawn_http_server(common::router(state)).await;

    let start_url = format!(
        "{}/api/providers/gemini/accounts/login/start",
        server.base_url
    );
    let start_resp = server
        .client
        .post(start_url)
        .json(&json!({}))
        .send()
        .await
        .expect("start gemini login request");
    assert_eq!(start_resp.status(), StatusCode::OK);
    let start_body: GeminiLoginStartResponse = start_resp.json().await.expect("start body");

    let status = poll_gemini_login_status(&server, &start_body.login_id).await;
    assert_eq!(status.status, "failed");
    assert!(status.account_id.is_none());
    assert!(status
        .error
        .unwrap_or_default()
        .contains("gemini auth failed"));

    let accounts_url = format!("{}/api/providers/gemini/accounts", server.base_url);
    let accounts_resp = server
        .client
        .get(accounts_url)
        .send()
        .await
        .expect("gemini accounts request");
    assert_eq!(accounts_resp.status(), StatusCode::OK);
    let accounts: SubscriptionAccountsResponse = accounts_resp.json().await.expect("accounts body");
    assert!(accounts.accounts.is_empty());
    assert!(accounts.active_account_id.is_none());
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
