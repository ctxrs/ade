mod common;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use ctx_core::models::SessionEventType;
use ctx_http::installer::{save_agent_server_config, AgentServerCommand, AgentServerConfigFile};
use ctx_providers::adapters::{
    ProviderAdapter, ProviderHealth, ProviderStatus, RunHandle, TurnInput,
};
use ctx_providers::events::NormalizedEvent;

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
struct GeminiLoginStartResponse {
    login_id: String,
    auth_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GeminiLoginStatusResponse {
    status: String,
    account_id: Option<String>,
    auth_url: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct KimiLoginStartResponse {
    login_id: String,
    auth_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct KimiLoginStatusResponse {
    status: String,
    account_id: Option<String>,
    auth_url: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Clone)]
enum GeminiLoginFixture {
    Success {
        oauth_creds_json: String,
        google_accounts_json: Option<String>,
        auth_url: Option<String>,
    },
    Failure {
        error: String,
    },
    NoAuthUrl,
}

#[derive(Debug, Clone)]
struct GeminiLoginTestAdapter {
    fixture: GeminiLoginFixture,
}

impl GeminiLoginTestAdapter {
    fn success(
        oauth_creds_json: impl Into<String>,
        google_accounts_json: Option<String>,
        auth_url: Option<String>,
    ) -> Self {
        Self {
            fixture: GeminiLoginFixture::Success {
                oauth_creds_json: oauth_creds_json.into(),
                google_accounts_json,
                auth_url,
            },
        }
    }

    fn failure(error: impl Into<String>) -> Self {
        Self {
            fixture: GeminiLoginFixture::Failure {
                error: error.into(),
            },
        }
    }

    fn no_auth_url() -> Self {
        Self {
            fixture: GeminiLoginFixture::NoAuthUrl,
        }
    }
}

#[async_trait]
impl ProviderAdapter for GeminiLoginTestAdapter {
    async fn inspect(&self) -> Result<ProviderStatus> {
        Ok(ProviderStatus {
            provider_id: "gemini".to_string(),
            installed: true,
            detected_path: None,
            version: Some("test".to_string()),
            capabilities: None,
            health: ProviderHealth::Ok,
            diagnostics: Vec::new(),
            details: HashMap::new(),
        })
    }

    async fn run(
        &self,
        _input: TurnInput,
        _workdir: PathBuf,
        _env: HashMap<String, String>,
        _event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<RunHandle> {
        Err(anyhow!("run is not used in this test adapter"))
    }

    async fn cancel(&self, _handle: RunHandle) -> Result<()> {
        Ok(())
    }

    async fn authenticate_session(
        &self,
        _session_key: String,
        _workdir: PathBuf,
        env: HashMap<String, String>,
        method_id: Option<String>,
        event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<()> {
        if method_id.as_deref() != Some("oauth-personal") {
            return Err(anyhow!(
                "unexpected method_id: {:?}",
                method_id.unwrap_or_default()
            ));
        }
        let Some(home) = env.get("GEMINI_CLI_HOME") else {
            return Err(anyhow!("GEMINI_CLI_HOME missing"));
        };
        let gemini_dir = PathBuf::from(home).join(".gemini");
        tokio::fs::create_dir_all(&gemini_dir).await?;

        match &self.fixture {
            GeminiLoginFixture::Success {
                oauth_creds_json,
                google_accounts_json,
                auth_url,
            } => {
                if let Some(auth_url) = auth_url.as_ref() {
                    let _ = event_sink
                        .send(NormalizedEvent {
                            event_type: SessionEventType::Notice,
                            payload_json: json!({ "auth_url": auth_url }),
                        })
                        .await;
                }
                tokio::fs::write(gemini_dir.join("oauth_creds.json"), oauth_creds_json).await?;
                if let Some(google_accounts_json) = google_accounts_json.as_ref() {
                    tokio::fs::write(
                        gemini_dir.join("google_accounts.json"),
                        google_accounts_json,
                    )
                    .await?;
                }
                Ok(())
            }
            GeminiLoginFixture::Failure { error } => {
                let _ = event_sink
                    .send(NormalizedEvent {
                        event_type: SessionEventType::Error,
                        payload_json: json!({ "message": error }),
                    })
                    .await;
                Err(anyhow!("{error}"))
            }
            GeminiLoginFixture::NoAuthUrl => {
                // Keep channel alive briefly and emit no auth URL or oauth files.
                tokio::time::sleep(Duration::from_millis(600)).await;
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone)]
enum KimiLoginFixture {
    Success {
        provider: String,
        credentials_json: String,
        config_toml: Option<String>,
        auth_url: Option<String>,
    },
    Failure {
        error: String,
    },
    NoAuthUrl,
}

#[derive(Debug, Clone)]
struct KimiLoginTestAdapter {
    fixture: KimiLoginFixture,
}

impl KimiLoginTestAdapter {
    fn success(
        provider: impl Into<String>,
        credentials_json: impl Into<String>,
        config_toml: Option<String>,
        auth_url: Option<String>,
    ) -> Self {
        Self {
            fixture: KimiLoginFixture::Success {
                provider: provider.into(),
                credentials_json: credentials_json.into(),
                config_toml,
                auth_url,
            },
        }
    }

    fn failure(error: impl Into<String>) -> Self {
        Self {
            fixture: KimiLoginFixture::Failure {
                error: error.into(),
            },
        }
    }

    fn no_auth_url() -> Self {
        Self {
            fixture: KimiLoginFixture::NoAuthUrl,
        }
    }
}

#[async_trait]
impl ProviderAdapter for KimiLoginTestAdapter {
    async fn inspect(&self) -> Result<ProviderStatus> {
        Ok(ProviderStatus {
            provider_id: "kimi".to_string(),
            installed: true,
            detected_path: None,
            version: Some("test".to_string()),
            capabilities: None,
            health: ProviderHealth::Ok,
            diagnostics: Vec::new(),
            details: HashMap::new(),
        })
    }

    async fn run(
        &self,
        _input: TurnInput,
        _workdir: PathBuf,
        _env: HashMap<String, String>,
        _event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<RunHandle> {
        Err(anyhow!("run is not used in this test adapter"))
    }

    async fn cancel(&self, _handle: RunHandle) -> Result<()> {
        Ok(())
    }

    async fn authenticate_session(
        &self,
        _session_key: String,
        _workdir: PathBuf,
        env: HashMap<String, String>,
        method_id: Option<String>,
        event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<()> {
        if method_id.is_some() && method_id.as_deref() != Some("oauth-personal") {
            return Err(anyhow!(
                "unexpected method_id: {:?}",
                method_id.unwrap_or_default()
            ));
        }
        let Some(share_dir_raw) = env.get("KIMI_SHARE_DIR") else {
            return Err(anyhow!("KIMI_SHARE_DIR missing"));
        };
        let Some(home_dir_raw) = env.get("HOME") else {
            return Err(anyhow!("HOME missing"));
        };
        let share_dir = PathBuf::from(share_dir_raw);
        let home_dir = PathBuf::from(home_dir_raw);
        if home_dir.join(".kimi") != share_dir {
            return Err(anyhow!(
                "KIMI_SHARE_DIR should equal HOME/.kimi, got HOME={} KIMI_SHARE_DIR={}",
                home_dir.display(),
                share_dir.display()
            ));
        }
        let credentials_dir = share_dir.join("credentials");
        tokio::fs::create_dir_all(&credentials_dir).await?;

        match &self.fixture {
            KimiLoginFixture::Success {
                provider,
                credentials_json,
                config_toml,
                auth_url,
            } => {
                if let Some(auth_url) = auth_url.as_ref() {
                    let _ = event_sink
                        .send(NormalizedEvent {
                            event_type: SessionEventType::Notice,
                            payload_json: json!({ "auth_url": auth_url }),
                        })
                        .await;
                }
                tokio::fs::write(
                    credentials_dir.join(format!("{provider}.json")),
                    credentials_json,
                )
                .await?;
                let config_contents = config_toml
                    .clone()
                    .unwrap_or_else(|| format!("current_provider = \"{provider}\"\n"));
                tokio::fs::write(share_dir.join("config.toml"), config_contents).await?;
                Ok(())
            }
            KimiLoginFixture::Failure { error } => {
                let _ = event_sink
                    .send(NormalizedEvent {
                        event_type: SessionEventType::Error,
                        payload_json: json!({ "message": error }),
                    })
                    .await;
                Err(anyhow!("{error}"))
            }
            KimiLoginFixture::NoAuthUrl => {
                tokio::time::sleep(Duration::from_millis(600)).await;
                Ok(())
            }
        }
    }
}

fn providers_with_gemini_adapter(
    adapter: Arc<dyn ProviderAdapter>,
) -> HashMap<String, Arc<dyn ProviderAdapter>> {
    let mut providers = common::fake_providers();
    providers.insert("gemini".to_string(), adapter);
    providers
}

fn providers_with_kimi_adapter(
    adapter: Arc<dyn ProviderAdapter>,
) -> HashMap<String, Arc<dyn ProviderAdapter>> {
    let mut providers = common::fake_providers();
    providers.insert("kimi".to_string(), adapter);
    providers
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

async fn write_mock_kimi_runtime(
    data_root: &std::path::Path,
    script_contents: &str,
) -> std::path::PathBuf {
    let script_path = data_root.join("mock-kimi");
    tokio::fs::write(&script_path, script_contents)
        .await
        .expect("write mock kimi script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&script_path)
            .expect("mock kimi metadata")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script_path, perms).expect("set mock kimi permissions");
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

async fn poll_gemini_login_status(
    server: &common::TestServer,
    login_id: &str,
) -> GeminiLoginStatusResponse {
    let status_url = format!(
        "{}/api/providers/gemini/accounts/login/{}",
        server.base_url, login_id
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let resp = server
            .client
            .get(&status_url)
            .send()
            .await
            .expect("gemini status request");
        assert_eq!(resp.status(), StatusCode::OK);
        let body: GeminiLoginStatusResponse = resp.json().await.expect("gemini status body");
        if body.status != "pending" {
            return body;
        }
        if Instant::now() >= deadline {
            panic!("gemini login did not complete in time");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn poll_kimi_login_status(
    server: &common::TestServer,
    login_id: &str,
) -> KimiLoginStatusResponse {
    let status_url = format!(
        "{}/api/providers/kimi/accounts/login/{}",
        server.base_url, login_id
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let resp = server
            .client
            .get(&status_url)
            .send()
            .await
            .expect("kimi status request");
        assert_eq!(resp.status(), StatusCode::OK);
        let body: KimiLoginStatusResponse = resp.json().await.expect("kimi status body");
        if body.status != "pending" {
            return body;
        }
        if Instant::now() >= deadline {
            panic!("kimi login did not complete in time");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
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
async fn gemini_login_fails_fast_when_no_auth_url_is_emitted() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let providers = providers_with_gemini_adapter(Arc::new(GeminiLoginTestAdapter::no_auth_url()));
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
    assert!(status
        .error
        .unwrap_or_default()
        .contains("did not emit an OAuth URL"));
}

#[tokio::test]
async fn kimi_login_start_and_status_success_persists_account() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let providers = providers_with_kimi_adapter(Arc::new(KimiLoginTestAdapter::success(
        "moonshot",
        r#"{"access_token":"access","refresh_token":"refresh"}"#,
        Some("current_provider = \"moonshot\"\n".to_string()),
        Some("https://www.moonshot.ai/oauth/authorize?code=test".to_string()),
    )));
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0",
    );
    let server = common::spawn_http_server(common::router(state)).await;

    let start_url = format!(
        "{}/api/providers/kimi/accounts/login/start",
        server.base_url
    );
    let start_resp = server
        .client
        .post(start_url)
        .json(&json!({ "label": "Kimi OAuth" }))
        .send()
        .await
        .expect("start kimi login request");
    assert_eq!(start_resp.status(), StatusCode::OK);
    let start_body: KimiLoginStartResponse = start_resp.json().await.expect("start body");
    assert!(!start_body.login_id.is_empty());
    assert!(start_body.auth_url.is_none());

    let status = poll_kimi_login_status(&server, &start_body.login_id).await;
    assert_eq!(status.status, "success");
    assert!(status.account_id.is_some());
    assert!(status.error.is_none());
    assert!(status.auth_url.as_deref().is_some());

    let accounts_url = format!("{}/api/providers/kimi/accounts", server.base_url);
    let accounts_resp = server
        .client
        .get(accounts_url)
        .send()
        .await
        .expect("kimi accounts request");
    assert_eq!(accounts_resp.status(), StatusCode::OK);
    let accounts: SubscriptionAccountsResponse = accounts_resp.json().await.expect("accounts body");
    assert_eq!(accounts.accounts.len(), 1);
    assert_eq!(accounts.active_account_id, status.account_id);
    assert_eq!(accounts.accounts[0].label.as_deref(), Some("Kimi OAuth"));
}

#[tokio::test]
async fn kimi_login_start_and_status_success_via_direct_cli_login_command() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let server = common::spawn_http_server(common::router(state)).await;

    let script_path = write_mock_kimi_runtime(
        data_dir.path(),
        r#"#!/usr/bin/env bash
set -euo pipefail
if [[ "${1-}" != "login" || "${2-}" != "--json" ]]; then
  echo "unexpected args: $*" >&2
  exit 2
fi
echo '{"type":"info","message":"Please visit the following URL to finish authorization."}'
echo '{"type":"verification_url","message":"Verification URL: https://www.kimi.com/code/authorize_device?user_code=TEST-CODE","data":{"verification_url":"https://www.kimi.com/code/authorize_device?user_code=TEST-CODE","user_code":"TEST-CODE"}}'
mkdir -p "$HOME/.kimi/credentials"
cat > "$HOME/.kimi/config.toml" <<'EOF'
current_provider = "moonshot"
EOF
cat > "$HOME/.kimi/credentials/moonshot.json" <<'EOF'
{"access_token":"access","refresh_token":"refresh"}
EOF
echo '{"type":"success","message":"Logged in successfully."}'
"#,
    )
    .await;

    let mut cfg = AgentServerConfigFile {
        providers: HashMap::new(),
        managed_installs: HashMap::new(),
    };
    cfg.providers.insert(
        "kimi".to_string(),
        AgentServerCommand {
            command: script_path.to_string_lossy().to_string(),
            args: vec!["--acp".to_string()],
            dependencies: vec![],
            managed: None,
        },
    );
    save_agent_server_config(data_dir.path(), &cfg)
        .await
        .expect("save agent config");

    let start_url = format!(
        "{}/api/providers/kimi/accounts/login/start",
        server.base_url
    );
    let start_resp = server
        .client
        .post(start_url)
        .json(&json!({ "label": "Kimi Direct OAuth" }))
        .send()
        .await
        .expect("start kimi login request");
    assert_eq!(start_resp.status(), StatusCode::OK);
    let start_body: KimiLoginStartResponse = start_resp.json().await.expect("start body");
    assert!(!start_body.login_id.is_empty());
    assert!(start_body.auth_url.is_none());

    let status = poll_kimi_login_status(&server, &start_body.login_id).await;
    assert_eq!(status.status, "success");
    assert!(status.account_id.is_some());
    assert!(status.error.is_none());
    assert_eq!(
        status.auth_url.as_deref(),
        Some("https://www.kimi.com/code/authorize_device?user_code=TEST-CODE")
    );
}

#[tokio::test]
async fn kimi_login_start_and_status_failure_reports_error() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let providers =
        providers_with_kimi_adapter(Arc::new(KimiLoginTestAdapter::failure("kimi auth failed")));
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0",
    );
    let server = common::spawn_http_server(common::router(state)).await;

    let start_url = format!(
        "{}/api/providers/kimi/accounts/login/start",
        server.base_url
    );
    let start_resp = server
        .client
        .post(start_url)
        .json(&json!({}))
        .send()
        .await
        .expect("start kimi login request");
    assert_eq!(start_resp.status(), StatusCode::OK);
    let start_body: KimiLoginStartResponse = start_resp.json().await.expect("start body");

    let status = poll_kimi_login_status(&server, &start_body.login_id).await;
    assert_eq!(status.status, "failed");
    assert!(status.account_id.is_none());
    assert!(status
        .error
        .unwrap_or_default()
        .contains("kimi auth failed"));

    let accounts_url = format!("{}/api/providers/kimi/accounts", server.base_url);
    let accounts_resp = server
        .client
        .get(accounts_url)
        .send()
        .await
        .expect("kimi accounts request");
    assert_eq!(accounts_resp.status(), StatusCode::OK);
    let accounts: SubscriptionAccountsResponse = accounts_resp.json().await.expect("accounts body");
    assert!(accounts.accounts.is_empty());
    assert!(accounts.active_account_id.is_none());
}

#[tokio::test]
async fn kimi_login_fails_fast_when_no_auth_url_is_emitted() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let providers = providers_with_kimi_adapter(Arc::new(KimiLoginTestAdapter::no_auth_url()));
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0",
    );
    let server = common::spawn_http_server(common::router(state)).await;

    let start_url = format!(
        "{}/api/providers/kimi/accounts/login/start",
        server.base_url
    );
    let start_resp = server
        .client
        .post(start_url)
        .json(&json!({}))
        .send()
        .await
        .expect("start kimi login request");
    assert_eq!(start_resp.status(), StatusCode::OK);
    let start_body: KimiLoginStartResponse = start_resp.json().await.expect("start body");

    let status = poll_kimi_login_status(&server, &start_body.login_id).await;
    assert_eq!(status.status, "failed");
    assert!(status
        .error
        .unwrap_or_default()
        .contains("did not emit an OAuth URL"));
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
