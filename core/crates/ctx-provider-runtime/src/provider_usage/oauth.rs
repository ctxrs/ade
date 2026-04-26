use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::json;

const CODEX_OAUTH_REFRESH_MAX_AGE_SECS: i64 = 8 * 24 * 60 * 60;

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

pub(super) async fn fetch_codex_usage_oauth(
    env: &HashMap<String, String>,
) -> Result<serde_json::Value> {
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
    let contents = tokio::fs::read_to_string(auth_path)
        .await
        .with_context(|| format!("reading codex auth.json at {}", auth_path.display()))?;
    let mut json = serde_json::from_str::<serde_json::Value>(&contents)
        .with_context(|| format!("invalid codex auth.json at {}", auth_path.display()))?;
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
    let contents = match tokio::fs::read_to_string(&config_path).await {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok("https://chatgpt.com/backend-api".to_string());
        }
        Err(err) => {
            return Err(err).with_context(|| {
                format!("reading codex config.toml at {}", config_path.display())
            });
        }
    };
    let config = toml::from_str::<CodexConfigFile>(&contents)
        .with_context(|| format!("invalid codex config.toml at {}", config_path.display()))?;
    if let Some(url) = config.chatgpt_base_url {
        return Ok(url);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn persist_codex_auth_tokens_fails_closed_on_malformed_auth_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let auth_path = temp.path().join("auth.json");
        tokio::fs::write(&auth_path, "{ not valid json")
            .await
            .expect("write invalid auth.json");

        let auth = CodexAuthFile {
            openai_api_key: None,
            tokens: Some(CodexAuthTokens {
                access_token: "access-token".to_string(),
                refresh_token: "refresh-token".to_string(),
                id_token: None,
                account_id: Some("acct-1".to_string()),
            }),
            last_refresh: None,
        };
        let refreshed = CodexAuthTokens {
            access_token: "new-access-token".to_string(),
            refresh_token: "new-refresh-token".to_string(),
            id_token: None,
            account_id: Some("acct-1".to_string()),
        };

        let err = persist_codex_auth_tokens(&auth_path, &auth, &refreshed)
            .await
            .expect_err("malformed codex auth.json should fail closed");
        let message = format!("{err:#}");
        assert!(
            message.contains("invalid codex auth.json"),
            "expected parse context in error: {message}"
        );
        assert!(
            message.contains("auth.json"),
            "expected auth path in error: {message}"
        );
    }

    #[tokio::test]
    async fn fetch_codex_usage_oauth_fails_closed_on_malformed_codex_config() {
        let temp = tempfile::tempdir().expect("tempdir");
        let auth_path = temp.path().join("auth.json");
        tokio::fs::write(
            &auth_path,
            serde_json::json!({
                "tokens": {
                    "access_token": "access-token",
                    "refresh_token": "refresh-token"
                }
            })
            .to_string(),
        )
        .await
        .expect("write auth.json");
        tokio::fs::write(temp.path().join("config.toml"), "chatgpt_base_url = [")
            .await
            .expect("write invalid config.toml");

        let env = HashMap::from([(
            "CODEX_HOME".to_string(),
            temp.path().to_string_lossy().to_string(),
        )]);
        let err = fetch_codex_usage_oauth(&env)
            .await
            .expect_err("malformed codex config.toml should fail closed");
        let message = format!("{err:#}");
        assert!(
            message.contains("invalid codex config.toml"),
            "expected parse context in error: {message}"
        );
        assert!(
            message.contains("config.toml"),
            "expected config path in error: {message}"
        );
    }
}
