use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use ctx_core::provider_policy::CODEX_APP_SERVER_ARGS;
use ctx_provider_accounts as provider_accounts;
use ctx_provider_install::install_state::InstallTarget;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

use crate::daemon::AppState;

const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(60);
const CODEX_RPC_TIMEOUT: Duration = Duration::from_secs(10);
const CODEX_OAUTH_REFRESH_MAX_AGE_SECS: i64 = 8 * 24 * 60 * 60;

#[derive(Debug, Clone, Serialize)]
pub struct ProviderUsageSnapshot {
    pub provider_id: String,
    pub source: String,
    pub fetched_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CodexAuthFile {
    #[serde(rename = "OPENAI_API_KEY")]
    openai_api_key: Option<String>,
    tokens: Option<CodexAuthTokens>,
    last_refresh: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CodexAuthTokens {
    access_token: String,
    refresh_token: String,
    id_token: Option<String>,
    account_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CodexConfigFile {
    chatgpt_base_url: Option<String>,
}

pub fn spawn_provider_usage_poller(state: Arc<AppState>) {
    let mut shutdown_rx = state.core.shutdown_tx.subscribe();
    let poll_interval = usage_poll_interval_from_env().unwrap_or(DEFAULT_POLL_INTERVAL);
    if poll_interval.is_zero() {
        return;
    }
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(poll_interval);
        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => break,
                _ = ticker.tick() => {
                    if let Err(err) = refresh_provider_usage(&state).await {
                        tracing::warn!("provider usage poll failed: {err:#}");
                    }
                }
            }
        }
    });
}

pub async fn refresh_provider_usage(state: &Arc<AppState>) -> Result<()> {
    let mut env = provider_accounts::codex_env_for_active_account(&state.core.data_root).await?;
    let cfg = crate::installer::load_agent_server_config(&state.core.data_root)
        .await
        .unwrap_or_default();
    crate::installer::ensure_codex_cli_command_env_for_target(
        &mut env,
        &cfg,
        "codex",
        Some(InstallTarget::Host),
    )?;
    refresh_provider_usage_for(state, "codex", env).await?;
    Ok(())
}

pub async fn refresh_provider_usage_for(
    state: &Arc<AppState>,
    provider_id: &str,
    env: HashMap<String, String>,
) -> Result<ProviderUsageSnapshot> {
    let snapshot = match provider_id {
        "codex" => fetch_codex_usage(env).await?,
        _ => {
            return Ok(ProviderUsageSnapshot {
                provider_id: provider_id.to_string(),
                source: "unsupported".to_string(),
                fetched_at: Utc::now(),
                payload: None,
                error: Some("usage not supported for provider".to_string()),
            });
        }
    };
    let mut cache = state.providers.usage_cache.lock().await;
    cache.insert(provider_id.to_string(), snapshot.clone());
    Ok(snapshot)
}

pub async fn fetch_codex_usage_snapshot(
    env: HashMap<String, String>,
) -> Result<ProviderUsageSnapshot> {
    fetch_codex_usage(env).await
}

async fn fetch_codex_usage(env: HashMap<String, String>) -> Result<ProviderUsageSnapshot> {
    match fetch_codex_usage_oauth(&env).await {
        Ok(payload) => Ok(ProviderUsageSnapshot {
            provider_id: "codex".to_string(),
            source: "oauth".to_string(),
            fetched_at: Utc::now(),
            payload: Some(payload),
            error: None,
        }),
        Err(err) => match fetch_codex_usage_rpc(&env).await {
            Ok(payload) => Ok(ProviderUsageSnapshot {
                provider_id: "codex".to_string(),
                source: "rpc".to_string(),
                fetched_at: Utc::now(),
                payload: Some(payload),
                error: None,
            }),
            Err(rpc_err) => Ok(ProviderUsageSnapshot {
                provider_id: "codex".to_string(),
                source: "error".to_string(),
                fetched_at: Utc::now(),
                payload: None,
                error: Some(format!("{err}; rpc fallback failed: {rpc_err}")),
            }),
        },
    }
}

async fn fetch_codex_usage_oauth(env: &HashMap<String, String>) -> Result<serde_json::Value> {
    let auth_path = resolve_codex_auth_path(env)?;
    let auth = load_codex_auth(&auth_path).await?;
    let tokens = auth
        .tokens
        .as_ref()
        .ok_or_else(|| anyhow!("codex auth.json missing tokens"))?;
    let access_token = tokens.access_token.clone();
    let mut account_id = tokens.account_id.clone();
    let refresh_token = tokens.refresh_token.clone();

    let mut base_url = read_codex_base_url(env).await?;
    base_url = base_url.trim_end_matches('/').to_string();
    let usage_url = if base_url.contains("/backend-api") {
        format!("{base_url}/wham/usage")
    } else {
        format!("{base_url}/api/codex/usage")
    };

    let client = reqwest::Client::new();
    match codex_usage_request(&client, &usage_url, &access_token, account_id.as_deref()).await {
        Ok(payload) => Ok(payload),
        Err(err) => {
            if let Some(mut refreshed) =
                try_refresh_codex_tokens(&client, &refresh_token, &auth_path, &auth).await?
            {
                account_id = refreshed.account_id.take();
                let payload = codex_usage_request(
                    &client,
                    &usage_url,
                    &refreshed.access_token,
                    account_id.as_deref(),
                )
                .await
                .context("codex oauth usage after refresh failed")?;
                return Ok(payload);
            }
            Err(err)
        }
    }
}

async fn codex_usage_request(
    client: &reqwest::Client,
    url: &str,
    access_token: &str,
    account_id: Option<&str>,
) -> Result<serde_json::Value> {
    let mut req = client.get(url).bearer_auth(access_token);
    req = req.header("User-Agent", "ctx");
    if let Some(account_id) = account_id {
        req = req.header("ChatGPT-Account-Id", account_id);
    }
    let resp = req.send().await.context("codex usage request failed")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let msg = if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            "codex usage unauthorized"
        } else {
            "codex usage request failed"
        };
        return Err(anyhow!("{msg}: {status}; body={body}"));
    }
    let payload = resp.json::<serde_json::Value>().await?;
    Ok(payload)
}

async fn try_refresh_codex_tokens(
    client: &reqwest::Client,
    refresh_token: &str,
    auth_path: &Path,
    auth: &CodexAuthFile,
) -> Result<Option<CodexAuthTokens>> {
    if refresh_token.is_empty() {
        return Ok(None);
    }
    if !should_refresh_codex_tokens(auth) {
        return Ok(None);
    }
    let resp = client
        .post("https://auth.openai.com/oauth/token")
        .json(&json!({
            "client_id": "app_EMoamEEZ73f0CkXaXp7hrann",
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
            "scope": "openid profile email",
        }))
        .send()
        .await
        .context("codex oauth refresh request failed")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("codex oauth refresh failed: {status}; body={body}"));
    }
    let payload = resp.json::<serde_json::Value>().await?;
    let access_token = payload
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("codex oauth refresh missing access_token"))?
        .to_string();
    let refresh_token = payload
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let id_token = payload
        .get("id_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let refreshed = CodexAuthTokens {
        access_token,
        refresh_token,
        id_token,
        account_id: auth.tokens.as_ref().and_then(|t| t.account_id.clone()),
    };
    persist_codex_auth_tokens(auth_path, auth, &refreshed).await?;
    Ok(Some(refreshed))
}

fn should_refresh_codex_tokens(auth: &CodexAuthFile) -> bool {
    let Some(last_refresh) = auth.last_refresh.as_deref() else {
        return true;
    };
    let Ok(dt) = DateTime::parse_from_rfc3339(last_refresh) else {
        return true;
    };
    let age = Utc::now().signed_duration_since(dt.with_timezone(&Utc));
    age.num_seconds() > CODEX_OAUTH_REFRESH_MAX_AGE_SECS
}

async fn persist_codex_auth_tokens(
    auth_path: &Path,
    auth: &CodexAuthFile,
    refreshed: &CodexAuthTokens,
) -> Result<()> {
    let mut json = serde_json::json!({});
    if let Ok(contents) = tokio::fs::read_to_string(auth_path).await {
        if let Ok(existing) = serde_json::from_str::<serde_json::Value>(&contents) {
            json = existing;
        }
    }
    if json.get("tokens").and_then(|v| v.as_object()).is_none() {
        json["tokens"] = serde_json::Value::Object(serde_json::Map::new());
    }
    let Some(tokens) = json.get_mut("tokens").and_then(|v| v.as_object_mut()) else {
        return Err(anyhow!(
            "failed to persist codex auth tokens: tokens object missing"
        ));
    };
    tokens.insert(
        "access_token".to_string(),
        serde_json::Value::String(refreshed.access_token.clone()),
    );
    tokens.insert(
        "refresh_token".to_string(),
        serde_json::Value::String(refreshed.refresh_token.clone()),
    );
    if let Some(id_token) = refreshed.id_token.as_ref() {
        tokens.insert(
            "id_token".to_string(),
            serde_json::Value::String(id_token.clone()),
        );
    }
    if let Some(account_id) = refreshed.account_id.as_ref() {
        tokens.insert(
            "account_id".to_string(),
            serde_json::Value::String(account_id.clone()),
        );
    }
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    json["last_refresh"] = serde_json::Value::String(now);
    if let Some(api_key) = auth.openai_api_key.as_ref() {
        json["OPENAI_API_KEY"] = serde_json::Value::String(api_key.clone());
    }
    let contents = serde_json::to_vec_pretty(&json)?;
    tokio::fs::write(auth_path, contents).await?;
    Ok(())
}

async fn load_codex_auth(auth_path: &Path) -> Result<CodexAuthFile> {
    let contents = tokio::fs::read_to_string(auth_path)
        .await
        .with_context(|| format!("missing codex auth.json at {}", auth_path.display()))?;
    let auth: CodexAuthFile = serde_json::from_str(&contents)?;
    if auth.tokens.is_none() && auth.openai_api_key.is_none() {
        return Err(anyhow!("codex auth.json has no tokens or api key"));
    }
    Ok(auth)
}

async fn read_codex_base_url(env: &HashMap<String, String>) -> Result<String> {
    let codex_home = resolve_codex_home(env)?;
    let config_path = codex_home.join("config.toml");
    if let Ok(contents) = tokio::fs::read_to_string(&config_path).await {
        if let Ok(config) = toml::from_str::<CodexConfigFile>(&contents) {
            if let Some(url) = config.chatgpt_base_url {
                return Ok(url);
            }
        }
    }
    Ok("https://chatgpt.com/backend-api".to_string())
}

fn lookup_env(env: &HashMap<String, String>, key: &str) -> Option<String> {
    env.get(key).cloned().or_else(|| std::env::var(key).ok())
}

fn resolve_codex_auth_path(env: &HashMap<String, String>) -> Result<PathBuf> {
    if let Some(path) = lookup_env(env, "CTX_CODEX_AUTH_PATH").filter(|v| !v.trim().is_empty()) {
        return Ok(PathBuf::from(path));
    }
    let codex_home = resolve_codex_home(env)?;
    Ok(codex_home.join("auth.json"))
}

fn resolve_codex_home(env: &HashMap<String, String>) -> Result<PathBuf> {
    if let Some(home) = lookup_env(env, "CODEX_HOME").filter(|v| !v.trim().is_empty()) {
        return Ok(PathBuf::from(home));
    }
    let base = directories::BaseDirs::new().ok_or_else(|| anyhow!("missing home dir"))?;
    Ok(base.home_dir().join(".codex"))
}

fn usage_poll_interval_from_env() -> Option<Duration> {
    let raw = std::env::var("CTX_PROVIDER_USAGE_INTERVAL_MS").ok()?;
    let ms: u64 = raw.trim().parse().ok()?;
    Some(Duration::from_millis(ms))
}

async fn fetch_codex_usage_rpc(env: &HashMap<String, String>) -> Result<serde_json::Value> {
    let mut child = spawn_codex_app_server(env)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("codex app-server stdout unavailable"))?;
    let mut reader = BufReader::new(stdout).lines();
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("codex app-server stdin unavailable"))?;

    let init_request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "clientInfo": {
                "name": "ctx",
                "title": "ctx",
                "version": env!("CARGO_PKG_VERSION")
            }
        }
    });
    send_jsonrpc(&mut stdin, &init_request).await?;
    wait_for_response(&mut reader, 1, CODEX_RPC_TIMEOUT).await?;
    send_jsonrpc(
        &mut stdin,
        &json!({"jsonrpc": "2.0", "method": "initialized"}),
    )
    .await?;

    let rate_limits_request = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "account/rateLimits/read"
    });
    send_jsonrpc(&mut stdin, &rate_limits_request).await?;
    let response = wait_for_response(&mut reader, 2, CODEX_RPC_TIMEOUT).await?;

    let payload = response
        .get("result")
        .and_then(|v| v.get("rateLimits"))
        .ok_or_else(|| anyhow!("codex rpc rate limits missing result"))?;
    let normalized = normalize_codex_rpc_rate_limits(payload)?;

    let _ = child.kill().await;
    Ok(normalized)
}

fn spawn_codex_app_server(env: &HashMap<String, String>) -> Result<Child> {
    let codex_bin = env
        .get("CTX_CODEX_BIN_PATH")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("CTX_CODEX_BIN_PATH must be set to an absolute codex-cli path"))?;
    if !Path::new(codex_bin).is_absolute() {
        anyhow::bail!("CTX_CODEX_BIN_PATH must be absolute, got `{codex_bin}`");
    }
    let mut cmd = Command::new(codex_bin);
    cmd.args(CODEX_APP_SERVER_ARGS)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env {
        cmd.env(key, value);
    }
    cmd.spawn()
        .with_context(|| format!("spawning codex app-server via `{codex_bin}`"))
}

async fn send_jsonrpc(
    stdin: &mut tokio::process::ChildStdin,
    value: &serde_json::Value,
) -> Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    stdin.write_all(&bytes).await?;
    stdin.flush().await?;
    Ok(())
}

async fn wait_for_response(
    reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    request_id: i64,
    timeout: Duration,
) -> Result<serde_json::Value> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(anyhow!("codex rpc timeout waiting for response"));
        }
        let line = tokio::time::timeout(remaining, reader.next_line())
            .await
            .context("codex rpc read timeout")??;
        let line = line.ok_or_else(|| anyhow!("codex rpc stdout closed"))?;
        let value: serde_json::Value = serde_json::from_str(&line)?;
        if let Some(id) = value.get("id").and_then(|v| v.as_i64()) {
            if id == request_id {
                if value.get("error").is_some() {
                    return Err(anyhow!("codex rpc error: {value}"));
                }
                return Ok(value);
            }
        }
    }
}

fn normalize_codex_rpc_rate_limits(rate_limits: &serde_json::Value) -> Result<serde_json::Value> {
    let plan_type = rate_limits
        .get("planType")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let primary = normalize_codex_rpc_window(rate_limits.get("primary"));
    let secondary = normalize_codex_rpc_window(rate_limits.get("secondary"));
    let credits = normalize_codex_rpc_credits(rate_limits.get("credits"));
    Ok(json!({
        "plan_type": plan_type,
        "rate_limit": {
            "primary_window": primary,
            "secondary_window": secondary,
        },
        "credits": credits,
    }))
}

fn normalize_codex_rpc_window(value: Option<&serde_json::Value>) -> Option<serde_json::Value> {
    let value = value?;
    if value.is_null() {
        return None;
    }
    let used_percent = value.get("usedPercent").and_then(|v| v.as_i64())?;
    let window_minutes = value.get("windowDurationMins").and_then(|v| v.as_i64());
    let resets_at = value.get("resetsAt").and_then(|v| v.as_i64());
    let limit_window_seconds = window_minutes.map(|mins| mins * 60);
    let reset_after_seconds = resets_at.map(|ts| (ts - Utc::now().timestamp()).max(0));
    Some(json!({
        "used_percent": used_percent,
        "limit_window_seconds": limit_window_seconds,
        "reset_after_seconds": reset_after_seconds,
        "reset_at": resets_at,
    }))
}

fn normalize_codex_rpc_credits(value: Option<&serde_json::Value>) -> Option<serde_json::Value> {
    let value = value?;
    if value.is_null() {
        return None;
    }
    Some(json!({
        "has_credits": value.get("hasCredits").and_then(|v| v.as_bool()).unwrap_or(false),
        "unlimited": value.get("unlimited").and_then(|v| v.as_bool()).unwrap_or(false),
        "balance": value.get("balance").cloned(),
    }))
}
