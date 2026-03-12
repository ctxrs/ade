use super::*;
use base64::Engine;
use ctx_core::provider_policy::CODEX_APP_SERVER_ARGS;
use rand_core::RngCore;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

mod browser;
mod codex;
mod kimi;
mod mistral;

pub(crate) use browser::{
    get_amp_login, get_gemini_login, get_qwen_login, start_amp_login, start_gemini_login,
    start_qwen_login,
};
pub(crate) use codex::{complete_codex_login, get_codex_login, start_codex_login};
pub(crate) use kimi::{get_kimi_login, start_kimi_login};
pub(crate) use mistral::{get_mistral_login, start_mistral_login};

#[derive(Debug, Deserialize)]
pub(crate) struct ClaudeLoginStartReq {
    label: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ClaudeLoginStartResp {
    login_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ClaudeLoginCompleteReq {
    callback_code: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ClaudeLoginCompleteResp {
    accepted: bool,
}

#[derive(Debug, Serialize)]
struct ClaudeOauthTokenExchangeReq<'a> {
    grant_type: &'a str,
    client_id: &'a str,
    code: &'a str,
    redirect_uri: &'a str,
    code_verifier: &'a str,
    state: &'a str,
}

#[derive(Debug, Deserialize)]
struct ClaudeOauthTokenExchangeResp {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
    #[serde(default)]
    scope: Option<String>,
}

const CODEX_LOGIN_RPC_TIMEOUT: Duration = Duration::from_secs(30);
const GEMINI_LOGIN_TIMEOUT_DEFAULT: Duration = Duration::from_secs(300);
const GEMINI_LOGIN_POLL_INTERVAL: Duration = Duration::from_millis(700);
const QWEN_LOGIN_TIMEOUT_DEFAULT: Duration = Duration::from_secs(300);
const QWEN_LOGIN_POLL_INTERVAL: Duration = Duration::from_millis(700);
const AMP_LOGIN_TIMEOUT_DEFAULT: Duration = Duration::from_secs(300);
const AMP_LOGIN_POLL_INTERVAL: Duration = Duration::from_millis(700);
const MISTRAL_LOGIN_TIMEOUT_DEFAULT: Duration = Duration::from_secs(300);
const MISTRAL_LOGIN_POLL_INTERVAL: Duration = Duration::from_millis(700);
const QWEN_OAUTH_AUTH_METHOD_ID: &str = "qwen-oauth";
const AMP_BROWSER_AUTH_METHOD_ID: &str = "amp_browser_login";
const CLAUDE_OAUTH_AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
const CLAUDE_OAUTH_TOKEN_URL: &str = "https://console.anthropic.com/v1/oauth/token";
const CLAUDE_OAUTH_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const CLAUDE_OAUTH_REDIRECT_URI: &str = "https://platform.claude.com/oauth/code/callback";
const CLAUDE_OAUTH_SCOPE: &str =
    "org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers";

fn is_loopback_host(value: &str) -> bool {
    let host = value.trim().to_ascii_lowercase();
    if host == "localhost" {
        return true;
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return ip.is_loopback();
    }
    false
}

pub(super) fn expected_callback_from_auth_url(auth_url: &str) -> Option<String> {
    let parsed = Url::parse(auth_url).ok()?;
    let redirect = parsed
        .query_pairs()
        .find_map(|(key, value)| (key == "redirect_uri").then_some(value.into_owned()))?;
    let callback = Url::parse(&redirect).ok()?;
    let host = callback.host_str()?;
    if !is_loopback_host(host) {
        return None;
    }
    Some(callback.to_string())
}

pub(super) fn validate_callback_url(
    callback_url: &str,
    expected_callback_url: Option<&str>,
) -> anyhow::Result<()> {
    let callback = Url::parse(callback_url)
        .with_context(|| format!("invalid callback_url: {callback_url}"))?;
    if callback.scheme() != "http" {
        anyhow::bail!("callback_url must use http scheme");
    }
    let host = callback
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("callback_url must include host"))?;
    if !is_loopback_host(host) {
        anyhow::bail!("callback_url host must be loopback");
    }
    if callback.port().is_none() {
        anyhow::bail!("callback_url must include explicit port");
    }
    if !callback.path().starts_with("/auth/callback") {
        anyhow::bail!("callback_url path must start with /auth/callback");
    }
    if callback.query().is_none() {
        anyhow::bail!("callback_url must include query parameters");
    }

    if let Some(expected_raw) = expected_callback_url {
        let expected = Url::parse(expected_raw)
            .with_context(|| format!("invalid expected callback URL: {expected_raw}"))?;
        if callback.port() != expected.port() {
            anyhow::bail!("callback_url port mismatch");
        }
        if callback.path() != expected.path() {
            anyhow::bail!("callback_url path mismatch");
        }
    }
    Ok(())
}

fn gemini_login_timeout() -> Duration {
    let seconds = std::env::var("CTX_GEMINI_LOGIN_TIMEOUT_SECS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(GEMINI_LOGIN_TIMEOUT_DEFAULT.as_secs());
    Duration::from_secs(seconds)
}

fn qwen_login_timeout() -> Duration {
    let seconds = std::env::var("CTX_QWEN_LOGIN_TIMEOUT_SECS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(QWEN_LOGIN_TIMEOUT_DEFAULT.as_secs());
    Duration::from_secs(seconds)
}

fn amp_login_timeout() -> Duration {
    let seconds = std::env::var("CTX_AMP_LOGIN_TIMEOUT_SECS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(AMP_LOGIN_TIMEOUT_DEFAULT.as_secs());
    Duration::from_secs(seconds)
}

fn mistral_login_timeout() -> Duration {
    let seconds = std::env::var("CTX_MISTRAL_LOGIN_TIMEOUT_SECS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(MISTRAL_LOGIN_TIMEOUT_DEFAULT.as_secs());
    Duration::from_secs(seconds)
}

fn generate_claude_oauth_random() -> String {
    let mut bytes = [0u8; 32];
    rand_core::OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn generate_claude_oauth_login_session(
    label: Option<String>,
) -> provider_accounts::ClaudeOauthLoginSession {
    provider_accounts::ClaudeOauthLoginSession {
        label,
        state: generate_claude_oauth_random(),
        code_verifier: generate_claude_oauth_random(),
        redirect_uri: CLAUDE_OAUTH_REDIRECT_URI.to_string(),
    }
}

fn claude_oauth_code_challenge(code_verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(code_verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hasher.finalize())
}

fn build_claude_oauth_auth_url(
    session: &provider_accounts::ClaudeOauthLoginSession,
) -> anyhow::Result<String> {
    let mut params = url::form_urlencoded::Serializer::new(String::new());
    params.append_pair("code", "true");
    params.append_pair("client_id", CLAUDE_OAUTH_CLIENT_ID);
    params.append_pair("response_type", "code");
    params.append_pair("redirect_uri", &session.redirect_uri);
    params.append_pair("scope", CLAUDE_OAUTH_SCOPE);
    params.append_pair(
        "code_challenge",
        &claude_oauth_code_challenge(&session.code_verifier),
    );
    params.append_pair("code_challenge_method", "S256");
    params.append_pair("state", &session.state);
    let query = params.finish();
    Url::parse(&format!("{CLAUDE_OAUTH_AUTHORIZE_URL}?{query}"))
        .map(|url| url.to_string())
        .context("building claude oauth authorize url")
}

fn normalize_claude_callback_code(callback_code: &str) -> String {
    let trimmed = callback_code.trim();
    if let Ok(url) = Url::parse(trimmed) {
        if let Some(code) = url.query_pairs().find_map(|(key, value)| {
            (key == "code")
                .then_some(value.trim().to_string())
                .filter(|candidate| !candidate.is_empty() && candidate != "true")
        }) {
            return code;
        }
        if let Some(fragment) = url.fragment() {
            let hash_params = url::form_urlencoded::parse(fragment.as_bytes());
            if let Some(code) = hash_params.into_iter().find_map(|(key, value)| {
                (key == "code")
                    .then_some(value.trim().to_string())
                    .filter(|candidate| !candidate.is_empty() && candidate != "true")
            }) {
                return code;
            }
        }
    }
    trimmed
        .split('#')
        .next()
        .unwrap_or(trimmed)
        .split('&')
        .next()
        .unwrap_or(trimmed)
        .trim()
        .to_string()
}

fn claude_oauth_token_url() -> String {
    // EXCEPTION: test hook for the local oauth token stub used by subscription account tests.
    std::env::var("CTX_CLAUDE_OAUTH_TOKEN_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| CLAUDE_OAUTH_TOKEN_URL.to_string())
}

async fn exchange_claude_oauth_callback_code(
    session: &provider_accounts::ClaudeOauthLoginSession,
    callback_code: &str,
) -> anyhow::Result<String> {
    let normalized_code = normalize_claude_callback_code(callback_code);
    if normalized_code.is_empty() {
        bail!("callback_code is required");
    }
    let response = reqwest::Client::new()
        .post(claude_oauth_token_url())
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::ACCEPT, "application/json, text/plain, */*")
        .header(reqwest::header::ORIGIN, "https://claude.ai")
        .header(reqwest::header::REFERER, "https://claude.ai/")
        .header(reqwest::header::USER_AGENT, "ctx claude oauth")
        .json(&ClaudeOauthTokenExchangeReq {
            grant_type: "authorization_code",
            client_id: CLAUDE_OAUTH_CLIENT_ID,
            code: &normalized_code,
            redirect_uri: &session.redirect_uri,
            code_verifier: &session.code_verifier,
            state: &session.state,
        })
        .send()
        .await
        .context("requesting claude oauth token exchange")?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!("claude oauth token exchange returned {}: {}", status, body);
    }
    let payload: ClaudeOauthTokenExchangeResp = response
        .json()
        .await
        .context("decoding claude oauth token exchange response")?;
    if payload.expires_in <= 0 {
        bail!("claude oauth token exchange returned non-positive expires_in");
    }
    let scopes = payload
        .scope
        .as_deref()
        .unwrap_or(CLAUDE_OAUTH_SCOPE)
        .split_whitespace()
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let expires_at =
        (Utc::now() + chrono::Duration::seconds(payload.expires_in)).timestamp_millis();
    Ok(serde_json::json!({
        "claudeAiOauth": {
            "accessToken": payload.access_token,
            "refreshToken": payload.refresh_token,
            "expiresAt": expires_at,
            "scopes": scopes,
        }
    })
    .to_string())
}

fn first_email_from_google_accounts(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            let email = map
                .get("email")
                .or_else(|| map.get("accountEmail"))
                .or_else(|| map.get("account_email"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|candidate| !candidate.is_empty())
                .map(ToString::to_string);
            email.or_else(|| map.values().find_map(first_email_from_google_accounts))
        }
        serde_json::Value::Array(values) => {
            values.iter().find_map(first_email_from_google_accounts)
        }
        _ => None,
    }
}

fn first_email_from_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            let direct = map
                .get("email")
                .or_else(|| map.get("accountEmail"))
                .or_else(|| map.get("account_email"))
                .or_else(|| map.get("user_email"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|candidate| !candidate.is_empty())
                .map(ToString::to_string);
            direct.or_else(|| map.values().find_map(first_email_from_value))
        }
        serde_json::Value::Array(values) => values.iter().find_map(first_email_from_value),
        _ => None,
    }
}

pub(super) fn extract_auth_url_from_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
                return Some(trimmed.to_string());
            }
            extract_auth_url(trimmed)
        }
        serde_json::Value::Object(map) => {
            let direct = [
                "auth_url",
                "authUrl",
                "url",
                "login_url",
                "loginUrl",
                "authorize_url",
            ]
            .into_iter()
            .find_map(|key| map.get(key))
            .and_then(extract_auth_url_from_value);
            direct.or_else(|| map.values().find_map(extract_auth_url_from_value))
        }
        serde_json::Value::Array(values) => values.iter().find_map(extract_auth_url_from_value),
        _ => None,
    }
}

fn auth_notice_code(payload: &serde_json::Value) -> &str {
    payload
        .get("code")
        .or_else(|| payload.get("kind"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
}

fn is_auth_success_notice_code(code: &str) -> bool {
    matches!(
        code,
        "auth_complete" | "auth_completed" | "auth_success" | "authenticated"
    )
}

fn is_auth_failure_notice_code(code: &str) -> bool {
    matches!(code, "auth_failed" | "auth_error")
}

pub(crate) async fn start_claude_login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ClaudeLoginStartReq>,
) -> Result<Json<ClaudeLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
    let login_id = uuid::Uuid::new_v4().to_string();
    let session = generate_claude_oauth_login_session(req.label);
    let auth_url = build_claude_oauth_auth_url(&session).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: e.to_string(),
            }),
        )
    })?;
    let _ = resolve_claude_login_runtime(&state).await.map_err(|e| {
        let msg = e.to_string();
        let status = if msg.contains("runtime_command_") {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        (status, Json(ApiErrorResp { error: msg }))
    })?;
    let status = provider_accounts::ClaudeLoginStatus {
        login_id: login_id.clone(),
        auth_url: Some(auth_url.clone()),
        status: "pending".to_string(),
        account_id: None,
        error: None,
    };
    {
        let mut map = state.providers.claude_login_sessions.lock().await;
        map.insert(login_id.clone(), status);
    }
    {
        let mut map = state.providers.claude_oauth_login_sessions.lock().await;
        map.insert(login_id.clone(), session);
    }
    Ok(Json(ClaudeLoginStartResp {
        login_id,
        auth_url: Some(auth_url),
    }))
}

pub(crate) async fn complete_claude_login(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<ClaudeLoginCompleteReq>,
) -> Result<Json<ClaudeLoginCompleteResp>, (StatusCode, Json<ApiErrorResp>)> {
    let callback_code = req.callback_code.trim();
    if callback_code.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "callback_code is required".to_string(),
            }),
        ));
    }
    {
        let sessions = state.providers.claude_login_sessions.lock().await;
        let Some(status) = sessions.get(&id) else {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "login not found".to_string(),
                }),
            ));
        };
        if status.status != "pending" {
            return Err((
                StatusCode::CONFLICT,
                Json(ApiErrorResp {
                    error: "login is no longer pending".to_string(),
                }),
            ));
        }
    }
    let session = {
        let map = state.providers.claude_oauth_login_sessions.lock().await;
        map.get(&id).cloned()
    }
    .ok_or_else(|| {
        (
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "login session is not accepting callback input".to_string(),
            }),
        )
    })?;
    let credentials_json = match exchange_claude_oauth_callback_code(&session, callback_code).await
    {
        Ok(credentials_json) => credentials_json,
        Err(err) => {
            let error = logs::redact_sensitive(&err.to_string());
            {
                let mut map = state.providers.claude_login_sessions.lock().await;
                if let Some(entry) = map.get_mut(&id) {
                    entry.status = "failed".to_string();
                    entry.error = Some(error.clone());
                }
            }
            {
                let mut map = state.providers.claude_oauth_login_sessions.lock().await;
                map.remove(&id);
            }
            return Err((StatusCode::BAD_GATEWAY, Json(ApiErrorResp { error })));
        }
    };
    let registry = match provider_accounts::add_claude_oauth_account(
        &state.core.data_root,
        session.label.clone(),
        credentials_json,
        None,
    )
    .await
    {
        Ok(registry) => registry,
        Err(err) => {
            let error = logs::redact_sensitive(&err.to_string());
            {
                let mut map = state.providers.claude_login_sessions.lock().await;
                if let Some(entry) = map.get_mut(&id) {
                    entry.status = "failed".to_string();
                    entry.error = Some(error.clone());
                }
            }
            {
                let mut map = state.providers.claude_oauth_login_sessions.lock().await;
                map.remove(&id);
            }
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp { error }),
            ));
        }
    };
    {
        let mut map = state.providers.claude_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&id) {
            entry.status = "success".to_string();
            entry.account_id = registry.active_account_id.clone();
            entry.error = None;
        }
    }
    {
        let mut map = state.providers.claude_oauth_login_sessions.lock().await;
        map.remove(&id);
    }
    restarts::restart_claude_providers_for_auth_change(&state, "claude auth updated").await;
    Ok(Json(ClaudeLoginCompleteResp { accepted: true }))
}

pub(crate) async fn get_claude_login(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::ClaudeLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
    let map = state.providers.claude_login_sessions.lock().await;
    let status = map.get(&id).cloned().ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "login not found".to_string(),
            }),
        )
    })?;
    Ok(Json(status))
}

pub(super) async fn resolve_runtime_provider_command_from_config(
    data_root: &std::path::Path,
    provider_id: &str,
) -> anyhow::Result<Option<installer::ProviderRuntimeCommand>> {
    let cfg = installer::load_agent_server_config(data_root)
        .await
        .context("loading agent server config")?;
    installer::resolve_runtime_provider_command(&cfg, provider_id)
        .with_context(|| format!("resolving runtime command for {provider_id}"))
}

pub(super) async fn resolve_provider_login_command_from_config(
    data_root: &std::path::Path,
    provider_id: &str,
) -> anyhow::Result<Option<std::path::PathBuf>> {
    let cfg = installer::load_agent_server_config(data_root)
        .await
        .context("loading agent server config")?;
    installer::resolve_provider_login_command(&cfg, provider_id)
        .with_context(|| format!("resolving prepared login command for {provider_id}"))
}

pub(super) async fn resolve_claude_login_runtime_from_config(
    data_root: &std::path::Path,
) -> anyhow::Result<installer::ProviderRuntimeCommand> {
    resolve_runtime_provider_command_from_config(data_root, "claude-cli")
        .await?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "runtime_command_missing: provider=claude-cli (bundle claude-cli or configure an absolute runtime command)"
            )
        })
}

async fn resolve_claude_login_runtime(
    state: &Arc<AppState>,
) -> anyhow::Result<installer::ProviderRuntimeCommand> {
    resolve_claude_login_runtime_from_config(&state.core.data_root).await
}

fn strip_ansi_sequences(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut idx = 0usize;
    while idx < chars.len() {
        let ch = chars[idx];
        if ch == '\u{1b}' {
            idx += 1;
            if idx < chars.len() {
                if chars[idx] == '[' {
                    idx += 1;
                    while idx < chars.len() {
                        let c = chars[idx];
                        idx += 1;
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                    continue;
                }
                if chars[idx] == ']' {
                    idx += 1;
                    let mut payload = String::new();
                    while idx < chars.len() {
                        let c = chars[idx];
                        if c == '\u{7}' {
                            idx += 1;
                            break;
                        }
                        if c == '\u{1b}' && (idx + 1) < chars.len() && chars[idx + 1] == '\\' {
                            idx += 2;
                            break;
                        }
                        payload.push(c);
                        idx += 1;
                    }
                    if let Some(url) = payload
                        .strip_prefix("8;;")
                        .filter(|value| !value.is_empty())
                    {
                        out.push(' ');
                        out.push_str(url);
                        out.push(' ');
                    }
                    continue;
                }
            }
            continue;
        }
        if ch != '\r' && ch != '\u{7}' {
            out.push(ch);
        }
        idx += 1;
    }
    out
}

#[cfg(test)]
pub(super) fn normalize_claude_login_line(line: &str) -> String {
    strip_ansi_sequences(line.trim_end_matches('\r'))
}

fn is_auth_url_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
        || matches!(
            ch,
            '-' | '.'
                | '_'
                | '~'
                | ':'
                | '/'
                | '?'
                | '#'
                | '['
                | ']'
                | '@'
                | '!'
                | '$'
                | '&'
                | '\''
                | '('
                | ')'
                | '*'
                | '+'
                | ','
                | ';'
                | '='
                | '%'
        )
}

fn matches_auth_url_scheme_from(chars: &[char], start: usize, scheme: &str) -> bool {
    let mut idx = start;
    for expected in scheme.chars() {
        while idx < chars.len() && chars[idx].is_whitespace() {
            idx += 1;
        }
        if idx >= chars.len() || chars[idx].to_ascii_lowercase() != expected {
            return false;
        }
        idx += 1;
    }
    true
}

fn find_auth_url_start(chars: &[char], from_idx: usize) -> Option<usize> {
    let mut idx = from_idx;
    while idx < chars.len() {
        if chars[idx].eq_ignore_ascii_case(&'h')
            && (matches_auth_url_scheme_from(chars, idx, "https://")
                || matches_auth_url_scheme_from(chars, idx, "http://"))
        {
            return Some(idx);
        }
        idx += 1;
    }
    None
}

fn auth_scheme_can_continue_across_break(current: &str, next_fragment: &str) -> bool {
    let compact_current: String = current
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    let compact_next: String = next_fragment
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    if compact_current.is_empty() || compact_next.is_empty() {
        return false;
    }
    let combined = format!("{compact_current}{compact_next}");
    ["https://", "http://"]
        .into_iter()
        .any(|scheme| scheme.starts_with(&combined))
}

fn should_continue_auth_url_after_break(current: &str, next_fragment: &str) -> bool {
    if next_fragment.is_empty() {
        return false;
    }
    if auth_scheme_can_continue_across_break(current, next_fragment) {
        return true;
    }
    if next_fragment.chars().any(|ch| !ch.is_ascii_alphanumeric()) {
        return true;
    }
    if next_fragment
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_digit())
    {
        return true;
    }
    matches!(
        current.chars().last(),
        Some('%' | ':' | '=' | '&' | '?' | '/' | '#' | '-' | '_')
    )
}

pub(super) fn extract_auth_url(text: &str) -> Option<String> {
    let normalized = strip_ansi_sequences(text);
    let chars: Vec<char> = normalized.chars().collect();
    let mut idx = 0usize;
    while idx < chars.len() {
        idx = find_auth_url_start(&chars, idx)?;
        let mut end = idx;
        let mut candidate = String::new();
        while end < chars.len() {
            let ch = chars[end];
            if is_auth_url_char(ch) {
                candidate.push(ch);
                end += 1;
                continue;
            }
            if ch.is_whitespace() {
                let mut probe = end;
                while probe < chars.len() && chars[probe].is_whitespace() {
                    probe += 1;
                }
                if probe < chars.len() && is_auth_url_char(chars[probe]) {
                    let mut token_end = probe;
                    while token_end < chars.len() && is_auth_url_char(chars[token_end]) {
                        token_end += 1;
                    }
                    let next_fragment: String = chars[probe..token_end].iter().collect();
                    if should_continue_auth_url_after_break(&candidate, &next_fragment) {
                        end = probe;
                        continue;
                    }
                }
            }
            break;
        }
        let trimmed = candidate
            .trim_matches(|c: char| {
                matches!(
                    c,
                    '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | ',' | ';' | '.'
                )
            })
            .to_string();
        if let Ok(parsed) = Url::parse(&trimmed) {
            if matches!(parsed.scheme(), "http" | "https") {
                return Some(trimmed);
            }
        }
        idx = end.saturating_add(1);
    }
    None
}

#[cfg(test)]
pub(super) fn auth_url_looks_complete(auth_url: &str) -> bool {
    let parsed = match Url::parse(auth_url) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let maybe_redirect = parsed
        .query_pairs()
        .find_map(|(key, value)| (key == "redirect_uri").then_some(value.into_owned()));
    let Some(redirect_uri) = maybe_redirect else {
        return true;
    };
    let redirect = match Url::parse(&redirect_uri) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let Some(host) = redirect.host_str() else {
        return false;
    };
    if !is_loopback_host(host) {
        return true;
    }
    redirect.port().is_some()
}

#[cfg(test)]
pub(super) async fn read_trailing_claude_login_lines(
    line_rx: &mut mpsc::UnboundedReceiver<String>,
    grace: Duration,
) -> Vec<String> {
    let mut lines = Vec::new();
    let mut deadline = Instant::now() + grace;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, line_rx.recv()).await {
            Ok(Some(line)) => {
                lines.push(line);
                deadline = Instant::now() + grace;
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }
    lines
}
