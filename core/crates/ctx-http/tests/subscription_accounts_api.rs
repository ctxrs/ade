mod common;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex as AsyncMutex};
use url::Url;

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

#[derive(Debug, Clone, Deserialize)]
struct ClaudeTokenExchangeRequest {
    grant_type: String,
    client_id: String,
    code: String,
    redirect_uri: String,
    code_verifier: String,
    state: String,
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
struct QwenLoginStartResponse {
    login_id: String,
    auth_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct QwenLoginStatusResponse {
    status: String,
    account_id: Option<String>,
    auth_url: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MistralLoginStartResponse {
    login_id: String,
    auth_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MistralLoginStatusResponse {
    status: String,
    auth_url: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AmpLoginStartResponse {
    login_id: String,
    auth_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AmpLoginStatusResponse {
    status: String,
    auth_url: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CursorLoginStartResponse {
    login_id: String,
    auth_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CursorLoginStatusResponse {
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
        if env
            .get("NO_BROWSER")
            .is_some_and(|value| value.eq_ignore_ascii_case("true"))
        {
            return Err(anyhow!("NO_BROWSER=true should not be set"));
        }
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
enum QwenLoginFixture {
    Success {
        oauth_creds_json: String,
        auth_url: Option<String>,
    },
}

#[derive(Debug, Clone)]
struct QwenLoginTestAdapter {
    fixture: QwenLoginFixture,
}

impl QwenLoginTestAdapter {
    fn success(oauth_creds_json: impl Into<String>, auth_url: Option<String>) -> Self {
        Self {
            fixture: QwenLoginFixture::Success {
                oauth_creds_json: oauth_creds_json.into(),
                auth_url,
            },
        }
    }
}

#[async_trait]
impl ProviderAdapter for QwenLoginTestAdapter {
    async fn inspect(&self) -> Result<ProviderStatus> {
        Ok(ProviderStatus {
            provider_id: "qwen".to_string(),
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
        if method_id.as_deref() != Some("qwen-oauth") {
            return Err(anyhow!(
                "unexpected method_id: {:?}",
                method_id.unwrap_or_default()
            ));
        }
        let Some(home) = env.get("HOME") else {
            return Err(anyhow!("HOME missing"));
        };
        let qwen_dir = PathBuf::from(home).join(".qwen");
        tokio::fs::create_dir_all(&qwen_dir).await?;

        match &self.fixture {
            QwenLoginFixture::Success {
                oauth_creds_json,
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
                tokio::fs::write(qwen_dir.join("oauth_creds.json"), oauth_creds_json).await?;
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone)]
enum MistralLoginFixture {
    Success {
        auth_url: Option<String>,
        email: Option<String>,
    },
}

#[derive(Debug, Clone)]
struct MistralLoginTestAdapter {
    fixture: MistralLoginFixture,
}

impl MistralLoginTestAdapter {
    fn success(auth_url: Option<String>, email: Option<String>) -> Self {
        Self {
            fixture: MistralLoginFixture::Success { auth_url, email },
        }
    }
}

#[async_trait]
impl ProviderAdapter for MistralLoginTestAdapter {
    async fn inspect(&self) -> Result<ProviderStatus> {
        Ok(ProviderStatus {
            provider_id: "mistral".to_string(),
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
        _method_id: Option<String>,
        event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<()> {
        if !env.contains_key("HOME") {
            return Err(anyhow!("HOME missing"));
        }
        match &self.fixture {
            MistralLoginFixture::Success { auth_url, email } => {
                if let Some(auth_url) = auth_url.as_ref() {
                    let _ = event_sink
                        .send(NormalizedEvent {
                            event_type: SessionEventType::Notice,
                            payload_json: json!({ "auth_url": auth_url }),
                        })
                        .await;
                }
                let _ = event_sink
                    .send(NormalizedEvent {
                        event_type: SessionEventType::Notice,
                        payload_json: json!({
                            "code": "auth_complete",
                            "email": email,
                        }),
                    })
                    .await;
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone)]
enum AmpLoginFixture {
    AuthRequired {
        auth_url: Option<String>,
        message: String,
    },
}

#[derive(Debug, Clone)]
struct AmpLoginTestAdapter {
    fixture: AmpLoginFixture,
}

impl AmpLoginTestAdapter {
    fn auth_required(auth_url: Option<String>, message: impl Into<String>) -> Self {
        Self {
            fixture: AmpLoginFixture::AuthRequired {
                auth_url,
                message: message.into(),
            },
        }
    }
}

#[async_trait]
impl ProviderAdapter for AmpLoginTestAdapter {
    async fn inspect(&self) -> Result<ProviderStatus> {
        Ok(ProviderStatus {
            provider_id: "amp".to_string(),
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
        if method_id.as_deref() != Some("amp_browser_login") {
            return Err(anyhow!(
                "unexpected method_id: {:?}",
                method_id.unwrap_or_default()
            ));
        }
        if !env.contains_key("HOME") {
            return Err(anyhow!("HOME missing"));
        }

        match &self.fixture {
            AmpLoginFixture::AuthRequired { auth_url, message } => {
                if let Some(auth_url) = auth_url.as_ref() {
                    let _ = event_sink
                        .send(NormalizedEvent {
                            event_type: SessionEventType::Notice,
                            payload_json: json!({ "auth_url": auth_url }),
                        })
                        .await;
                }
                let _ = event_sink
                    .send(NormalizedEvent {
                        event_type: SessionEventType::Notice,
                        payload_json: json!({
                            "code": "auth_required",
                            "message": message,
                        }),
                    })
                    .await;
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

fn providers_with_qwen_adapter(
    adapter: Arc<dyn ProviderAdapter>,
) -> HashMap<String, Arc<dyn ProviderAdapter>> {
    let mut providers = common::fake_providers();
    providers.insert("qwen".to_string(), adapter);
    providers
}

fn providers_with_mistral_adapter(
    adapter: Arc<dyn ProviderAdapter>,
) -> HashMap<String, Arc<dyn ProviderAdapter>> {
    let mut providers = common::fake_providers();
    providers.insert("mistral".to_string(), adapter);
    providers
}

fn providers_with_amp_adapter(
    adapter: Arc<dyn ProviderAdapter>,
) -> HashMap<String, Arc<dyn ProviderAdapter>> {
    let mut providers = common::fake_providers();
    providers.insert("amp".to_string(), adapter);
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

async fn write_mock_cursor_runtime(
    data_root: &std::path::Path,
    script_contents: &str,
) -> std::path::PathBuf {
    let script_path = data_root.join("cursor-agent");
    tokio::fs::write(&script_path, script_contents)
        .await
        .expect("write mock cursor script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&script_path)
            .expect("mock cursor metadata")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script_path, perms).expect("set mock cursor permissions");
    }
    script_path
}

async fn write_mock_security_runtime(data_root: &std::path::Path) -> std::path::PathBuf {
    let bin_dir = data_root.join("mock-security-bin");
    tokio::fs::create_dir_all(&bin_dir)
        .await
        .expect("create mock security bin");
    let script_path = bin_dir.join("security");
    tokio::fs::write(&script_path, "#!/usr/bin/env bash\nexit 0\n")
        .await
        .expect("write mock security script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&script_path)
            .expect("mock security metadata")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script_path, perms).expect("set mock security permissions");
    }
    bin_dir
}

const MOCK_CLAUDE_OAUTH_CREDENTIALS_JSON: &str = r#"{"claudeAiOauth":{"accessToken":"access-token","refreshToken":"refresh-token","expiresAt":4102444800000,"scopes":["user:inference","user:profile"],"subscriptionType":"pro"}}"#;
const MOCK_CLAUDE_CONFIG_JSON: &str = r#"{"oauthAccount":{"emailAddress":"contact-086a332885a5@fixture.example.test","organizationUuid":"org-test","organizationName":"Profound App"}} "#;
static CLAUDE_TOKEN_ENV_LOCK: AsyncMutex<()> = AsyncMutex::const_new(());

struct TestEnvVar {
    key: &'static str,
    prev: Option<String>,
}

impl TestEnvVar {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, prev }
    }
}

impl Drop for TestEnvVar {
    fn drop(&mut self) {
        match self.prev.as_deref() {
            Some(value) => unsafe {
                std::env::set_var(self.key, value);
            },
            None => unsafe {
                std::env::remove_var(self.key);
            },
        }
    }
}

async fn start_claude_token_exchange_server(
    status: StatusCode,
    response_body: serde_json::Value,
) -> (
    String,
    Arc<Mutex<Vec<ClaudeTokenExchangeRequest>>>,
    tokio::task::JoinHandle<()>,
) {
    let captured = Arc::new(Mutex::new(Vec::<ClaudeTokenExchangeRequest>::new()));
    let captured_clone = Arc::clone(&captured);
    let app = axum::Router::new().route(
        "/v1/oauth/token",
        axum::routing::post(
            move |axum::Json(payload): axum::Json<ClaudeTokenExchangeRequest>| {
                let captured = Arc::clone(&captured_clone);
                let response_body = response_body.clone();
                async move {
                    captured.lock().expect("capture lock").push(payload);
                    (status, axum::Json(response_body))
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind claude token server");
    let addr = listener.local_addr().expect("claude token addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve claude token server");
    });
    (format!("http://{addr}/v1/oauth/token"), captured, handle)
}

fn parse_claude_auth_url(start_body: &ClaudeLoginStartResponse) -> Url {
    Url::parse(
        start_body
            .auth_url
            .as_deref()
            .expect("claude login auth url should be present"),
    )
    .expect("claude auth url")
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

async fn poll_qwen_login_status(
    server: &common::TestServer,
    login_id: &str,
) -> QwenLoginStatusResponse {
    let status_url = format!(
        "{}/api/providers/qwen/accounts/login/{}",
        server.base_url, login_id
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let resp = server
            .client
            .get(&status_url)
            .send()
            .await
            .expect("qwen status request");
        assert_eq!(resp.status(), StatusCode::OK);
        let body: QwenLoginStatusResponse = resp.json().await.expect("qwen status body");
        if body.status != "pending" {
            return body;
        }
        if Instant::now() >= deadline {
            panic!("qwen login did not complete in time");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn poll_mistral_login_status(
    server: &common::TestServer,
    login_id: &str,
) -> MistralLoginStatusResponse {
    let status_url = format!(
        "{}/api/providers/mistral/accounts/login/{}",
        server.base_url, login_id
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let resp = server
            .client
            .get(&status_url)
            .send()
            .await
            .expect("mistral status request");
        assert_eq!(resp.status(), StatusCode::OK);
        let body: MistralLoginStatusResponse = resp.json().await.expect("mistral status body");
        if body.status != "pending" {
            return body;
        }
        if Instant::now() >= deadline {
            panic!("mistral login did not complete in time");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn poll_amp_login_status(
    server: &common::TestServer,
    login_id: &str,
) -> AmpLoginStatusResponse {
    let status_url = format!(
        "{}/api/providers/amp/accounts/login/{}",
        server.base_url, login_id
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let resp = server
            .client
            .get(&status_url)
            .send()
            .await
            .expect("amp status request");
        assert_eq!(resp.status(), StatusCode::OK);
        let body: AmpLoginStatusResponse = resp.json().await.expect("amp status body");
        if body.status != "pending" {
            return body;
        }
        if Instant::now() >= deadline {
            panic!("amp login did not complete in time");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn poll_cursor_login_status(
    server: &common::TestServer,
    login_id: &str,
) -> CursorLoginStatusResponse {
    let status_url = format!(
        "{}/api/providers/cursor/accounts/login/{}",
        server.base_url, login_id
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let resp = server
            .client
            .get(&status_url)
            .send()
            .await
            .expect("cursor status request");
        assert_eq!(resp.status(), StatusCode::OK);
        let body: CursorLoginStatusResponse = resp.json().await.expect("cursor status body");
        if body.status != "pending" {
            return body;
        }
        if Instant::now() >= deadline {
            panic!("cursor login did not complete in time");
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
async fn claude_login_start_returns_pending_pkce_session() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let server = common::spawn_http_server(common::router(state)).await;

    let script_path =
        write_mock_claude_runtime(data_dir.path(), "#!/usr/bin/env bash\nexit 0\n").await;
    let mut cfg = AgentServerConfigFile {
        providers: HashMap::new(),
        managed_installs: HashMap::new(),
        managed_provider_targets: HashMap::new(),
        managed_install_targets: HashMap::new(),
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
    let auth_url = parse_claude_auth_url(&start_body);
    assert_eq!(auth_url.scheme(), "https");
    assert_eq!(auth_url.host_str(), Some("claude.ai"));
    assert_eq!(auth_url.path(), "/oauth/authorize");
    assert_eq!(
        auth_url
            .query_pairs()
            .find(|(k, _)| k == "code")
            .map(|(_, v)| v.into_owned())
            .as_deref(),
        Some("true")
    );
    assert_eq!(
        auth_url
            .query_pairs()
            .find(|(k, _)| k == "client_id")
            .map(|(_, v)| v.into_owned())
            .as_deref(),
        Some("9d1c250a-e61b-44d9-88ed-5944d1962f5e")
    );
    assert_eq!(
        auth_url
            .query_pairs()
            .find(|(k, _)| k == "response_type")
            .map(|(_, v)| v.into_owned())
            .as_deref(),
        Some("code")
    );
    assert_eq!(
        auth_url
            .query_pairs()
            .find(|(k, _)| k == "redirect_uri")
            .map(|(_, v)| v.into_owned())
            .as_deref(),
        Some("https://platform.claude.com/oauth/code/callback")
    );
    assert_eq!(
        auth_url
            .query_pairs()
            .find(|(k, _)| k == "code_challenge_method")
            .map(|(_, v)| v.into_owned())
            .as_deref(),
        Some("S256")
    );
    assert!(auth_url
        .query_pairs()
        .any(|(k, v)| k == "scope" && v.contains("user:sessions:claude_code")));
    assert!(auth_url
        .query_pairs()
        .any(|(k, v)| k == "state" && !v.is_empty()));
    assert!(auth_url
        .query_pairs()
        .any(|(k, v)| k == "code_challenge" && !v.is_empty()));

    let status_url = format!(
        "{}/api/providers/claude-crp/accounts/login/{}",
        server.base_url, start_body.login_id
    );
    let status_resp = server
        .client
        .get(status_url)
        .send()
        .await
        .expect("claude login status request");
    assert_eq!(status_resp.status(), StatusCode::OK);
    let status: ClaudeLoginStatusResponse =
        status_resp.json().await.expect("claude login status body");
    assert_eq!(status.status, "pending");
    assert!(status.account_id.is_none());
    assert!(status.error.is_none());
}

#[tokio::test]
async fn claude_login_start_requires_prepared_runtime_resolution() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let server = common::spawn_http_server(common::router(state)).await;

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
    assert_eq!(start_resp.status(), StatusCode::BAD_REQUEST);
    let body: ErrorResp = start_resp.json().await.expect("start error body");
    assert!(body
        .error
        .contains("runtime_command_missing: provider=claude-cli"));
}

#[tokio::test]
async fn claude_login_builds_hosted_pkce_auth_url() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let server = common::spawn_http_server(common::router(state)).await;

    let script_path =
        write_mock_claude_runtime(data_dir.path(), "#!/usr/bin/env bash\nexit 0\n").await;
    let mut cfg = AgentServerConfigFile {
        providers: HashMap::new(),
        managed_installs: HashMap::new(),
        managed_provider_targets: HashMap::new(),
        managed_install_targets: HashMap::new(),
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
    let auth_url = parse_claude_auth_url(&start_body);
    assert_eq!(
        auth_url
            .query_pairs()
            .find(|(key, _)| key == "redirect_uri")
            .map(|(_, value)| value.into_owned())
            .as_deref(),
        Some("https://platform.claude.com/oauth/code/callback")
    );
    assert!(auth_url
        .query_pairs()
        .any(|(key, value)| key == "scope" && value.contains("user:mcp_servers")));
}

#[tokio::test]
async fn claude_login_callback_code_completion_path_succeeds() {
    let _env_lock = CLAUDE_TOKEN_ENV_LOCK.lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let server = common::spawn_http_server(common::router(state)).await;
    let (token_url, captured, token_handle) = start_claude_token_exchange_server(
        StatusCode::OK,
        json!({
            "access_token": "access-token",
            "refresh_token": "refresh-token",
            "expires_in": 3600,
            "scope": "org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers"
        }),
    )
    .await;
    let _token_url = TestEnvVar::set("CTX_CLAUDE_OAUTH_TOKEN_URL", &token_url);
    let script_path =
        write_mock_claude_runtime(data_dir.path(), "#!/usr/bin/env bash\nexit 0\n").await;
    let mut cfg = AgentServerConfigFile {
        providers: HashMap::new(),
        managed_installs: HashMap::new(),
        managed_provider_targets: HashMap::new(),
        managed_install_targets: HashMap::new(),
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
    let auth_url = parse_claude_auth_url(&start_body);
    let expected_state = auth_url
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .expect("claude auth url state");

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

    let captured = captured.lock().expect("captured token request");
    assert_eq!(captured.len(), 1);
    let request = &captured[0];
    assert_eq!(request.grant_type, "authorization_code");
    assert_eq!(request.client_id, "9d1c250a-e61b-44d9-88ed-5944d1962f5e");
    assert_eq!(request.code, "ePBMdWetJlSbZ0aR");
    assert_eq!(
        request.redirect_uri,
        "https://platform.claude.com/oauth/code/callback"
    );
    assert_eq!(request.state, expected_state);
    assert!(!request.code_verifier.is_empty());
    drop(captured);
    token_handle.abort();
}

#[tokio::test]
async fn claude_login_token_exchange_failure_reports_actionable_error() {
    let _env_lock = CLAUDE_TOKEN_ENV_LOCK.lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let server = common::spawn_http_server(common::router(state)).await;
    let (token_url, _captured, token_handle) =
        start_claude_token_exchange_server(StatusCode::BAD_REQUEST, json!({ "error": "bad_code" }))
            .await;
    let _token_url = TestEnvVar::set("CTX_CLAUDE_OAUTH_TOKEN_URL", &token_url);
    let script_path =
        write_mock_claude_runtime(data_dir.path(), "#!/usr/bin/env bash\nexit 0\n").await;
    let mut cfg = AgentServerConfigFile {
        providers: HashMap::new(),
        managed_installs: HashMap::new(),
        managed_provider_targets: HashMap::new(),
        managed_install_targets: HashMap::new(),
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
    let complete_url = format!(
        "{}/api/providers/claude-crp/accounts/login/{}",
        server.base_url, start_body.login_id
    );
    let complete_resp = server
        .client
        .post(&complete_url)
        .json(&json!({ "callback_code": "bad-code" }))
        .send()
        .await
        .expect("complete claude login request");
    assert_eq!(complete_resp.status(), StatusCode::BAD_GATEWAY);
    let error_body: ErrorResp = complete_resp.json().await.expect("complete error body");
    assert!(error_body.error.contains("token exchange returned 400"));

    let status =
        poll_claude_login_status(&server, &start_body.login_id, Duration::from_secs(2)).await;
    assert_eq!(status.status, "failed");
    assert!(status.account_id.is_none());
    assert!(status
        .error
        .unwrap_or_default()
        .contains("token exchange returned 400"));
    token_handle.abort();
}

#[tokio::test]
async fn claude_login_complete_rejects_unknown_login() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let server = common::spawn_http_server(common::router(state)).await;
    let complete_url = format!(
        "{}/api/providers/claude-crp/accounts/login/unknown-login",
        server.base_url
    );
    let complete_resp = server
        .client
        .post(&complete_url)
        .json(&json!({ "callback_code": "missing" }))
        .send()
        .await
        .expect("complete claude login request");
    assert_eq!(complete_resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn claude_login_without_label_preserves_existing_account_label() {
    let _env_lock = CLAUDE_TOKEN_ENV_LOCK.lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let server = common::spawn_http_server(common::router(state)).await;

    let existing_body = ctx_http::provider_accounts::add_claude_oauth_account(
        data_dir.path(),
        Some("Claude Existing Label".to_string()),
        MOCK_CLAUDE_OAUTH_CREDENTIALS_JSON.to_string(),
        Some(MOCK_CLAUDE_CONFIG_JSON.to_string()),
    )
    .await
    .expect("create existing claude oauth account");
    let existing_account = existing_body
        .accounts
        .first()
        .expect("existing account should be present");
    let existing_id = existing_account.id.clone();
    assert_eq!(existing_account.label.as_str(), "Claude Existing Label");
    let (token_url, _captured, token_handle) = start_claude_token_exchange_server(
        StatusCode::OK,
        json!({
            "access_token": "access-token",
            "refresh_token": "refresh-token",
            "expires_in": 3600,
            "scope": "user:inference user:profile"
        }),
    )
    .await;
    let _token_url = TestEnvVar::set("CTX_CLAUDE_OAUTH_TOKEN_URL", &token_url);
    let script_path =
        write_mock_claude_runtime(data_dir.path(), "#!/usr/bin/env bash\nexit 0\n").await;
    let accounts_url = format!("{}/api/providers/claude-crp/accounts", server.base_url);
    let mut cfg = AgentServerConfigFile {
        providers: HashMap::new(),
        managed_installs: HashMap::new(),
        managed_provider_targets: HashMap::new(),
        managed_install_targets: HashMap::new(),
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
    let complete_url = format!(
        "{}/api/providers/claude-crp/accounts/login/{}",
        server.base_url, start_body.login_id
    );
    let complete_resp = server
        .client
        .post(&complete_url)
        .json(&json!({ "callback_code": "existing-account-code" }))
        .send()
        .await
        .expect("complete claude login request");
    assert_eq!(complete_resp.status(), StatusCode::OK);

    let status =
        poll_claude_login_status(&server, &start_body.login_id, Duration::from_secs(4)).await;
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
    assert_eq!(
        listed_body.accounts[0].label.as_deref(),
        Some("Claude Existing Label")
    );
    token_handle.abort();
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
async fn amp_login_auth_required_notice_reports_real_message() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let providers = providers_with_amp_adapter(Arc::new(AmpLoginTestAdapter::auth_required(
        Some("https://ampcode.com/auth".to_string()),
        "Amp needs subscription approval",
    )));
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0",
    );
    let server = common::spawn_http_server(common::router(state)).await;

    let start_url = format!("{}/api/providers/amp/accounts/login/start", server.base_url);
    let start_resp = server
        .client
        .post(start_url)
        .json(&json!({}))
        .send()
        .await
        .expect("start amp login request");
    assert_eq!(start_resp.status(), StatusCode::OK);
    let start_body: AmpLoginStartResponse = start_resp.json().await.expect("start body");
    assert!(!start_body.login_id.is_empty());
    assert!(start_body.auth_url.is_none());

    let status = poll_amp_login_status(&server, &start_body.login_id).await;
    assert_eq!(status.status, "failed");
    assert_eq!(
        status.error.as_deref(),
        Some("Amp needs subscription approval")
    );
    assert_eq!(status.auth_url.as_deref(), Some("https://ampcode.com/auth"));
}

#[tokio::test]
async fn qwen_login_start_and_status_success_persists_account() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let providers = providers_with_qwen_adapter(Arc::new(QwenLoginTestAdapter::success(
        r#"{"access_token":"access","refresh_token":"refresh"}"#,
        Some("https://chat.qwen.ai/oauth/authorize?code=test".to_string()),
    )));
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0",
    );
    let server = common::spawn_http_server(common::router(state)).await;

    let start_url = format!(
        "{}/api/providers/qwen/accounts/login/start",
        server.base_url
    );
    let start_resp = server
        .client
        .post(start_url)
        .json(&json!({ "label": "Qwen OAuth" }))
        .send()
        .await
        .expect("start qwen login request");
    assert_eq!(start_resp.status(), StatusCode::OK);
    let start_body: QwenLoginStartResponse = start_resp.json().await.expect("start body");
    assert!(!start_body.login_id.is_empty());
    assert!(start_body.auth_url.is_none());

    let status = poll_qwen_login_status(&server, &start_body.login_id).await;
    assert_eq!(status.status, "success");
    assert!(status.account_id.is_some());
    assert!(status.error.is_none());
    assert!(status.auth_url.as_deref().is_some());

    let accounts_url = format!("{}/api/providers/qwen/accounts", server.base_url);
    let accounts_resp = server
        .client
        .get(accounts_url)
        .send()
        .await
        .expect("qwen accounts request");
    assert_eq!(accounts_resp.status(), StatusCode::OK);
    let accounts: SubscriptionAccountsResponse = accounts_resp.json().await.expect("accounts body");
    assert_eq!(accounts.accounts.len(), 1);
    assert_eq!(accounts.active_account_id, status.account_id);
    assert_eq!(accounts.accounts[0].label.as_deref(), Some("Qwen OAuth"));
}

#[tokio::test]
async fn qwen_subscription_accounts_crud_round_trip() {
    assert_managed_subscription_crud(
        "qwen",
        json!({
            "label": "Qwen Managed",
            "oauth_creds_json": "{\"access_token\":\"a\",\"refresh_token\":\"b\"}",
            "email": "dev@example.com"
        }),
    )
    .await;
}

#[tokio::test]
async fn cursor_login_start_requires_cursor_agent_runtime() {
    let _env_lock = CLAUDE_TOKEN_ENV_LOCK.lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let server = common::spawn_http_server(common::router(state)).await;
    let _path_guard = TestEnvVar::set("PATH", data_dir.path().to_string_lossy().as_ref());

    let start_url = format!(
        "{}/api/providers/cursor/accounts/login/start",
        server.base_url
    );
    let start_resp = server
        .client
        .post(start_url)
        .json(&json!({}))
        .send()
        .await
        .expect("start cursor login request");
    assert_eq!(start_resp.status(), StatusCode::BAD_REQUEST);
    let body: ErrorResp = start_resp.json().await.expect("start error body");
    assert!(body
        .error
        .contains("runtime_command_missing: provider=cursor-login"));
}

#[tokio::test]
async fn cursor_login_start_and_status_success_persists_account() {
    let _env_lock = CLAUDE_TOKEN_ENV_LOCK.lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let server = common::spawn_http_server(common::router(state)).await;

    let cursor_script = write_mock_cursor_runtime(
        data_dir.path(),
        r#"#!/usr/bin/env node
const cp = require('child_process');
console.log('https://cursor.com/login/device?code=test');
console.log('Signed in as cursor-dev@example.com');
cp.spawnSync('security', ['add-generic-password', '-s', 'cursor-access-token', '-w', 'cursor-access-token'], { stdio: 'ignore' });
cp.spawnSync('security', ['add-generic-password', '-s', 'cursor-refresh-token', '-w', 'cursor-refresh-token'], { stdio: 'ignore' });
"#,
    )
    .await;
    let security_bin = write_mock_security_runtime(data_dir.path()).await;
    let existing_path = std::env::var("PATH").unwrap_or_default();
    let combined_path = format!("{}:{}", security_bin.to_string_lossy(), existing_path);
    let _path_guard = TestEnvVar::set("PATH", &combined_path);

    let mut cfg = AgentServerConfigFile {
        providers: HashMap::new(),
        managed_installs: HashMap::new(),
        managed_provider_targets: HashMap::new(),
        managed_install_targets: HashMap::new(),
    };
    cfg.providers.insert(
        "cursor".to_string(),
        AgentServerCommand {
            command: cursor_script.to_string_lossy().to_string(),
            args: vec!["--experimental-acp".to_string()],
            dependencies: vec![],
            managed: None,
        },
    );
    save_agent_server_config(data_dir.path(), &cfg)
        .await
        .expect("save agent config");

    let start_url = format!(
        "{}/api/providers/cursor/accounts/login/start",
        server.base_url
    );
    let start_resp = server
        .client
        .post(start_url)
        .json(&json!({ "label": "Cursor OAuth" }))
        .send()
        .await
        .expect("start cursor login request");
    assert_eq!(start_resp.status(), StatusCode::OK);
    let start_body: CursorLoginStartResponse = start_resp.json().await.expect("start body");
    assert!(!start_body.login_id.is_empty());
    assert!(start_body.auth_url.is_none());

    let status = poll_cursor_login_status(&server, &start_body.login_id).await;
    assert_eq!(status.status, "success");
    assert!(status.account_id.is_some());
    assert_eq!(
        status.auth_url.as_deref(),
        Some("https://cursor.com/login/device?code=test")
    );
    assert!(status.error.is_none());

    let accounts_url = format!("{}/api/providers/cursor/accounts", server.base_url);
    let accounts_resp = server
        .client
        .get(accounts_url)
        .send()
        .await
        .expect("cursor accounts request");
    assert_eq!(accounts_resp.status(), StatusCode::OK);
    let accounts: SubscriptionAccountsResponse = accounts_resp.json().await.expect("accounts body");
    assert_eq!(accounts.accounts.len(), 1);
    assert_eq!(accounts.active_account_id, status.account_id);
    assert_eq!(accounts.accounts[0].label.as_deref(), Some("Cursor OAuth"));
}

#[tokio::test]
async fn mistral_login_start_and_status_success_persists_account() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = common::setup_store(data_dir.path()).await;
    let providers = providers_with_mistral_adapter(Arc::new(MistralLoginTestAdapter::success(
        Some("https://auth.mistral.ai/oauth/authorize?code=test".to_string()),
        Some("mistral-dev@example.com".to_string()),
    )));
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0",
    );
    let server = common::spawn_http_server(common::router(state)).await;

    let start_url = format!(
        "{}/api/providers/mistral/accounts/login/start",
        server.base_url
    );
    let start_resp = server
        .client
        .post(start_url)
        .json(&json!({ "label": "Mistral OAuth" }))
        .send()
        .await
        .expect("start mistral login request");
    assert_eq!(start_resp.status(), StatusCode::OK);
    let start_body: MistralLoginStartResponse = start_resp.json().await.expect("start body");
    assert!(!start_body.login_id.is_empty());
    assert!(start_body.auth_url.is_none());

    let status = poll_mistral_login_status(&server, &start_body.login_id).await;
    assert_eq!(status.status, "success");
    assert!(status.error.is_none());
    assert!(status.auth_url.is_none());

    let accounts_url = format!("{}/api/providers/mistral/accounts", server.base_url);
    let accounts_resp = server
        .client
        .get(accounts_url)
        .send()
        .await
        .expect("mistral accounts request");
    assert_eq!(accounts_resp.status(), StatusCode::OK);
    let accounts: SubscriptionAccountsResponse = accounts_resp.json().await.expect("accounts body");
    assert_eq!(accounts.accounts.len(), 1);
    assert!(accounts.active_account_id.is_some());
    assert_eq!(accounts.accounts[0].label.as_deref(), Some("Mistral OAuth"));
}

#[tokio::test]
async fn mistral_subscription_accounts_crud_round_trip() {
    assert_managed_subscription_crud(
        "mistral",
        json!({
            "label": "Mistral Managed",
            "email": "dev@example.com"
        }),
    )
    .await;
}

#[tokio::test]
async fn amp_subscription_accounts_crud_round_trip() {
    assert_managed_subscription_crud(
        "amp",
        json!({
            "label": "Amp Managed",
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
